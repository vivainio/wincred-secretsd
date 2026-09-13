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
mkdir -p ~/.config/systemd/user
cp systemd/wincred-secretsd.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wincred-secretsd
```

This requires a persistent D-Bus session bus, which in turn requires
systemd running inside WSL (`systemd=true` in `/etc/wsl.conf` -- see
[wincred's README](https://github.com/vivainio/wincred#why-not-just-use-a-linux-keyring-under-wsl)
for why that's worth having anyway). Without it, a bare WSL shell has no
session bus for the daemon to claim `org.freedesktop.secrets` on.

If GNOME Keyring or KWallet is also running and already owns
`org.freedesktop.secrets` (unlikely in a headless WSL distro, more likely if
you've installed a desktop environment via WSLg), disable its secrets
component so this daemon can claim the name instead.

## How it maps onto wincred

Secret Service items are `(label, attributes: dict, secret bytes)`. wincred
credentials are `(target: string, username, secret)`. This daemon packs the
former into the latter: each item is stored at wincred target
`secretservice/<collection>/<uuid>`, with `{label, attributes, content_type,
secret_b64}` JSON-encoded as the wincred secret blob.

Set `WINCRED_EXE` to override the `wincred.exe` binary invoked (defaults to
`wincred.exe` on `PATH`) -- useful for testing against a stub.

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
