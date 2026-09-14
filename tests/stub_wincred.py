#!/usr/bin/env python3
"""Stand-in for wincred.exe, for testing wincred-secretsd without Windows.

Implements just enough of `wincred.exe {get,set,delete,list}` -- the same
CLI surface src/backend.rs shells out to -- backed by a JSON file instead of
the real Windows Credential Manager. Point WINCRED_EXE at this script and
WINCRED_STUB_STORE at a scratch file to use it.
"""
import json
import os
import sys


def load(store_path):
    if not os.path.exists(store_path):
        return {}
    with open(store_path) as f:
        return json.load(f)


def save(store_path, data):
    with open(store_path, "w") as f:
        json.dump(data, f)


def main():
    store_path = os.environ["WINCRED_STUB_STORE"]
    args = sys.argv[1:]
    if not args:
        sys.exit(2)
    cmd, rest = args[0], args[1:]
    data = load(store_path)

    if cmd == "get":
        target = rest[0]
        entry = data.get(target)
        if entry is None:
            sys.exit(1)  # backend.rs: exit 1 => Ok(None)
        print(json.dumps({"secret": entry["secret"]}))
        return

    if cmd == "set":
        target = rest[0]
        username = rest[rest.index("--user") + 1] if "--user" in rest else ""
        secret = sys.stdin.read()
        data[target] = {"username": username, "secret": secret}
        save(store_path, data)
        return

    if cmd == "delete":
        target = rest[0]
        if target not in data:
            sys.exit(1)  # backend.rs: exit 1 => Ok(false)
        del data[target]
        save(store_path, data)
        return

    if cmd == "list":
        prefix = rest[rest.index("--prefix") + 1] if "--prefix" in rest else ""
        entries = [{"target": t} for t in data if t.startswith(prefix)]
        print(json.dumps(entries))
        return

    sys.exit(2)


if __name__ == "__main__":
    main()
