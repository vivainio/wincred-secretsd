# wincred-secretsd

A [freedesktop.org Secret Service](https://specifications.freedesktop.org/secret-service/latest/)
D-Bus daemon for WSL, backed by the Windows Credential Manager via
[`wincred.exe`](https://github.com/vivainio/wincred). It lets any Linux
application that talks to "the keyring" -- `secret-tool`, `python-keyring`'s
`SecretService` backend, `git-credential-libsecret`, browsers -- read and
write secrets through Windows' already-unlocked credential vault instead of
GNOME Keyring/KWallet's separate, GUI-prompting master password.

Companion project to [wincred](https://github.com/vivainio/wincred); this
daemon shells out to `wincred.exe` for every actual read/write, the same way
you would from a shell script.

## Status

Early / experimental. The core item lifecycle works and has been exercised
against `secret-tool` (store, lookup, search, replace, clear) and via manual
D-Bus calls (`OpenSession`, `SearchItems`, `GetSecret`, `GetSecrets`,
property reads). Not yet implemented:

- Multiple/custom collections (`Service.CreateCollection`) -- only one fixed
  collection (`login`, aliased as `default`) exists.
- Real prompts -- everything is unlocked by construction (Windows already
  authenticated the session), so `Prompt` objects are never actually needed;
  calls that would return one return the well-known no-op path `/`.
- Real `Created`/`Modified` item timestamps (wincred doesn't track these).
- A "passthrough" collection exposing existing `wincred` targets (e.g. ones
  written by `git-credential-manager` on the Windows side) as Secret Service
  items -- see [Design notes](#design-notes).

## Install

Requires `wincred.exe` to already be set up and on `PATH` from WSL --
see [wincred's README](https://github.com/vivainio/wincred#install).

```sh
cargo install --path .
wincred-secretsd install
```

`install` writes a systemd user unit (pointing at wherever `cargo install`
put the binary, with `WINCRED_EXE` pinned to the resolved absolute path of
`wincred.exe` -- systemd user services get a minimal `PATH` that doesn't
include WSL's Windows interop entries, so a bare `wincred.exe` lookup at
runtime would fail even though it works from an interactive shell) and
enables it. It doesn't start anything or touch any currently-running
service -- restart WSL afterwards (`wsl --shutdown` from Windows, then
reopen a WSL window) so everything comes up fresh from the files `install`
wrote. Run `wincred-secretsd uninstall` to remove it again. Pass `--dry` to
either command to print what it would do -- which files it'd write/remove
and which `systemctl` commands it'd run -- without touching anything.

This requires a persistent D-Bus session bus, which in turn requires
systemd running inside WSL (`systemd=true` in `/etc/wsl.conf` -- see
[wincred's README](https://github.com/vivainio/wincred#why-not-just-use-a-linux-keyring-under-wsl)
for why that's worth having anyway). Without it, a bare WSL shell has no
session bus for the daemon to claim `org.freedesktop.secrets` on.

If GNOME Keyring is also installed and already owns
`org.freedesktop.secrets` (unlikely in a headless WSL distro, more likely if
you've installed a desktop environment via WSLg), `install` detects it,
first backs up its actual secret storage (`~/.local/share/keyrings/*` --
the `.keyring` files, not just config) to
`~/.local/share/wincred-secretsd/backups/keyrings-<timestamp>/` via a plain
`cp -a` (the original is left in place, untouched), then writes a systemd
override dropping just the `secrets` component from
`gnome-keyring-daemon.service` (keeping `pkcs11`/other components it was
running) -- so once WSL restarts, gnome-keyring-daemon comes up without
`secrets` and this daemon can claim the name uncontested, no manual edit
needed. `uninstall` removes that override so GNOME Keyring's original
components come back on the next restart (it doesn't touch the backup --
delete it yourself once you're confident everything's fine). Note none of
this migrates anything to wincred-secretsd's own storage: secrets GNOME
Keyring already had (saved browser passwords, `git-credential-libsecret`
entries, etc.) aren't moved to Windows Credential Manager -- those apps
just won't see them through the
Secret Service API anymore until re-saved. KWallet isn't handled
automatically; free the name
from it by hand first if that's what's running.

## How it maps onto wincred

Secret Service items are `(label, attributes: dict, secret bytes)`. wincred
credentials are `(target: string, username, secret)`. This daemon packs the
former into the latter: each item is stored at wincred target
`secretservice/<collection>/<uuid>`, with `{label, attributes, content_type,
secret_b64}` JSON-encoded as the wincred secret blob.

Set `WINCRED_EXE` to override the `wincred.exe` binary invoked (defaults to
`wincred.exe` on `PATH`) -- useful for testing against a stub.

## Testing

`tests/integration.py` drives the real daemon end-to-end via `secret-tool`
(store, lookup, search, replace, clear) against `tests/stub_wincred.py`, a
Python stand-in for `wincred.exe` backed by a JSON file instead of the
Windows Credential Manager. It starts its own private D-Bus session bus, so
it's safe to run alongside a real session and doesn't need Windows/wincred
at all -- just `cargo`, `dbus-launch`, and `secret-tool` (`apt install
dbus-user-session libsecret-tools`):

```sh
python3 tests/integration.py
```

## Design notes

Packing everything into opaque `secretservice/...` targets means an item
created here isn't visible to Windows-side tools by target name, unlike
`wincred`'s own cross-tool-sharing story (a `git-credential-manager` token on
Windows and a `wincred.exe get` from WSL are the same entry). A second,
read/write "passthrough" collection that exposes existing `wincred` targets
directly (with `attributes = {"target": name}`) would restore that, at the
cost of not supporting arbitrary attribute schemas for those items. Not
implemented yet.

## License

MIT
