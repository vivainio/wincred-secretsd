#!/usr/bin/env python3
"""End-to-end test for wincred-secretsd, runnable from a plain WSL shell.

Builds the daemon, starts a private D-Bus session bus (so this doesn't
touch your real session bus or GNOME Keyring), runs the daemon against the
stub backend in stub_wincred.py (tests/stub_wincred.py) instead of the real
wincred.exe, and drives it with `secret-tool` the same way a real client
would: store, lookup, search, replace, clear.

Usage:
    python3 tests/integration.py

Requires: cargo, dbus-launch, secret-tool (all present on a normal WSL
Ubuntu install: `apt install dbus-user-session libsecret-tools`).
"""
import json
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DAEMON_BIN = REPO_ROOT / "target" / "debug" / "wincred-secretsd"
STUB = Path(__file__).resolve().parent / "stub_wincred.py"

failures = []


def step(msg):
    print(f"-- {msg}")


def check(cond, msg):
    if cond:
        print(f"   ok: {msg}")
    else:
        print(f"   FAIL: {msg}")
        failures.append(msg)


def run(cmd, env, input=None, check_rc=True):
    result = subprocess.run(
        cmd, env=env, input=input, text=True, capture_output=True
    )
    if check_rc and result.returncode != 0:
        raise RuntimeError(
            f"{' '.join(cmd)} exited {result.returncode}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
    return result


def require_tools():
    for tool in ("cargo", "dbus-launch", "secret-tool"):
        if shutil.which(tool) is None:
            sys.exit(f"missing required tool: {tool}")


def start_private_bus():
    out = subprocess.check_output(["dbus-launch", "--sh-syntax"]).decode()
    addr = re.search(r"DBUS_SESSION_BUS_ADDRESS='?([^;']+)'?;", out).group(1)
    pid = int(re.search(r"DBUS_SESSION_BUS_PID=(\d+);", out).group(1))
    return addr, pid


def wait_for_ready(proc, timeout=10):
    """Block until the daemon's startup line hits stderr, or it dies."""
    deadline = timeout
    while True:
        r, _, _ = select.select([proc.stderr], [], [], deadline)
        if not r:
            raise RuntimeError("daemon did not become ready in time")
        line = proc.stderr.readline()
        if not line:
            raise RuntimeError("daemon exited before becoming ready")
        if "serving org.freedesktop.secrets" in line:
            return


def main():
    require_tools()

    step("cargo build")
    subprocess.run(["cargo", "build", "--quiet"], cwd=REPO_ROOT, check=True)
    if not DAEMON_BIN.exists():
        sys.exit(f"expected daemon binary at {DAEMON_BIN}")

    step("starting private D-Bus session bus")
    bus_addr, bus_pid = start_private_bus()

    with tempfile.TemporaryDirectory() as tmp:
        store_path = Path(tmp) / "wincred-stub-store.json"
        env = os.environ.copy()
        env["DBUS_SESSION_BUS_ADDRESS"] = bus_addr
        env["WINCRED_EXE"] = str(STUB)
        env["WINCRED_STUB_STORE"] = str(store_path)

        step("starting wincred-secretsd against the stub backend")
        daemon = subprocess.Popen(
            [str(DAEMON_BIN)], env=env, stderr=subprocess.PIPE, text=True
        )
        try:
            wait_for_ready(daemon)

            step("secret-tool store")
            run(
                ["secret-tool", "store", "--label=Test Item", "service", "testsvc", "username", "alice"],
                env=env,
                input="hunter2",
            )
            store = json.loads(store_path.read_text())
            check(len(store) == 1, f"one credential written to the stub store (got {len(store)})")

            step("secret-tool lookup")
            got = run(["secret-tool", "lookup", "service", "testsvc", "username", "alice"], env=env).stdout
            check(got == "hunter2", f"lookup returns the stored secret (got {got!r})")

            step("secret-tool search")
            found = run(["secret-tool", "search", "service", "testsvc"], env=env).stdout
            check("label = Test Item" in found, "search finds the item by attribute")

            step("secret-tool store (replace)")
            run(
                ["secret-tool", "store", "--label=Test Item", "service", "testsvc", "username", "alice"],
                env=env,
                input="new-secret",
            )
            store = json.loads(store_path.read_text())
            check(len(store) == 1, f"replace overwrites in place rather than duplicating (got {len(store)} entries)")
            got = run(["secret-tool", "lookup", "service", "testsvc", "username", "alice"], env=env).stdout
            check(got == "new-secret", f"lookup reflects the replaced secret (got {got!r})")

            step("a second, distinct item")
            run(
                ["secret-tool", "store", "--label=Other Item", "service", "othersvc"],
                env=env,
                input="other-secret",
            )
            store = json.loads(store_path.read_text())
            check(len(store) == 2, f"unrelated attributes create a separate item (got {len(store)} entries)")
            found = run(["secret-tool", "search", "service", "testsvc"], env=env).stdout
            check("label = Other Item" not in found, "search with attributes doesn't cross-match the other item")

            step("secret-tool clear")
            run(["secret-tool", "clear", "service", "testsvc", "username", "alice"], env=env)
            store = json.loads(store_path.read_text())
            check(len(store) == 1, f"clear removes just the targeted item (got {len(store)} entries)")

            step("lookup after clear")
            cleared = run(["secret-tool", "lookup", "service", "testsvc", "username", "alice"], env=env, check_rc=False)
            check(cleared.returncode != 0, "lookup fails once the item is cleared")

        finally:
            step("tearing down")
            daemon.send_signal(signal.SIGTERM)
            try:
                daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
            try:
                os.kill(bus_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    if failures:
        print(f"\n{len(failures)} check(s) failed:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("\nall checks passed")


if __name__ == "__main__":
    main()
