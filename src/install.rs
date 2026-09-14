use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const UNIT_NAME: &str = "wincred-secretsd.service";
const BUS_NAME: &str = "org.freedesktop.secrets";

fn unit_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

fn systemctl(args: &[&str]) -> Result<bool> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("spawning systemctl (is systemd running as PID 1? see README#install)")?;
    Ok(status.success())
}

/// A systemctl call that actually changes something -- printed and skipped
/// instead of run when `dry` is set.
fn systemctl_apply(dry: bool, args: &[&str]) -> Result<bool> {
    if dry {
        println!("[dry-run] would run: systemctl --user {}", args.join(" "));
        return Ok(true);
    }
    systemctl(args)
}

/// Write `contents` to `path` (creating parent dirs), or just print what
/// would be written when `dry` is set.
fn write_file(dry: bool, path: &Path, contents: &str) -> Result<()> {
    if dry {
        let indented: String = contents.lines().map(|l| format!("    {l}\n")).collect();
        println!("[dry-run] would write {}:\n{indented}", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Remove `path`, or just print that it would be removed when `dry` is set.
fn remove_file(dry: bool, path: &Path) -> Result<()> {
    if dry {
        println!("[dry-run] would remove {}", path.display());
        return Ok(());
    }
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
}

/// The effective ExecStart= line for a systemd --user unit, or None if the
/// unit doesn't exist. `systemctl cat` concatenates the vendor unit with any
/// drop-ins, so the *last* ExecStart= line is the one that actually applies.
fn unit_exec_start(unit: &str) -> Option<String> {
    let out = Command::new("systemctl")
        .args(["--user", "cat", unit])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ExecStart="))
        .rfind(|rest| !rest.is_empty())
        .map(str::to_string)
}

/// Rewrite a gnome-keyring-daemon-style `ExecStart=... --components="a,b,c" ...`
/// line with "secrets" dropped from the component list, preserving
/// everything else about the line (binary path, other flags, quoting).
/// Returns None if the line doesn't look like that shape.
fn without_secrets_component(exec_start: &str) -> Option<String> {
    let marker = "--components=";
    let marker_at = exec_start.find(marker)?;
    let after = &exec_start[marker_at + marker.len()..];
    let quoted = after.starts_with('"');
    let value = if quoted { &after[1..] } else { after };
    let value_end = if quoted {
        value.find('"')?
    } else {
        value.find(char::is_whitespace).unwrap_or(value.len())
    };
    let components: Vec<&str> = value[..value_end]
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "secrets")
        .collect();
    if components.is_empty() {
        return None; // nothing left to run the daemon for
    }
    let tail_at = marker_at + marker.len() + usize::from(quoted) + value_end + usize::from(quoted);
    let mut out = String::new();
    out.push_str(&exec_start[..marker_at]);
    out.push_str(marker);
    if quoted {
        out.push('"');
    }
    out.push_str(&components.join(","));
    if quoted {
        out.push('"');
    }
    out.push_str(&exec_start[tail_at..]);
    Some(out)
}

/// Copy GNOME Keyring's actual secret storage (the `.keyring` files backing
/// every secret it's ever held, as opposed to the systemd config this module
/// otherwise touches) somewhere safe before reconfiguring it -- so there's a
/// real fallback if something here goes wrong, independent of whether the
/// systemd override itself is correct. A plain `cp -a`, never a move or
/// delete; the original is left exactly where GNOME Keyring still expects
/// it.
fn backup_gnome_keyring_data(dry: bool) -> Result<()> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let src = PathBuf::from(&home).join(".local/share/keyrings");
    if !src.exists() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = PathBuf::from(&home)
        .join(".local/share/wincred-secretsd/backups")
        .join(format!("keyrings-{stamp}"));

    if dry {
        println!(
            "[dry-run] would back up {} to {} before reconfiguring gnome-keyring-daemon",
            src.display(),
            dest.display()
        );
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let status = Command::new("cp")
        .arg("-a")
        .arg(&src)
        .arg(&dest)
        .status()
        .context("spawning cp to back up GNOME Keyring's secret storage")?;
    if !status.success() {
        bail!("backing up {} to {} failed", src.display(), dest.display());
    }
    println!(
        "backed up GNOME Keyring's secret storage ({}) to {} before reconfiguring it",
        src.display(),
        dest.display()
    );
    Ok(())
}

/// If gnome-keyring-daemon.service is installed and configured with a
/// "secrets" component, drop just that component via a systemd override so
/// it stops competing for org.freedesktop.secrets -- keeping any other
/// components (pkcs11, ssh) it was running. No-op if the unit doesn't exist
/// or doesn't currently claim the secrets component.
async fn disable_gnome_keyring_secrets(dry: bool) -> Result<()> {
    const UNIT: &str = "gnome-keyring-daemon.service";
    let Some(exec_start) = unit_exec_start(UNIT) else {
        return Ok(()); // not installed on this system
    };
    if !exec_start.contains("secrets") {
        return Ok(());
    }

    backup_gnome_keyring_data(dry)?;

    let new_exec_start = without_secrets_component(&exec_start).unwrap_or_else(|| {
        eprintln!(
            "warning: couldn't parse {UNIT}'s ExecStart components list ({exec_start:?}) -- \
             falling back to a plain pkcs11-only ExecStart"
        );
        "/usr/bin/gnome-keyring-daemon --foreground --components=\"pkcs11\" \
         --control-directory=%t/keyring"
            .to_string()
    });

    let dropin_path = unit_dir()?
        .join(format!("{UNIT}.d"))
        .join("wincred-secretsd.conf");
    write_file(
        dry,
        &dropin_path,
        &format!("[Service]\nExecStart=\nExecStart={new_exec_start}\n"),
    )?;
    if !dry {
        println!("{UNIT} will no longer claim the \"secrets\" component once WSL is restarted");
    }
    Ok(())
}

fn unit_contents(exe: &Path, wincred_exe: Option<&Path>) -> String {
    let env_line = match wincred_exe {
        Some(p) => format!("Environment=WINCRED_EXE={}\n", p.display()),
        None => String::new(),
    };
    format!(
        "[Unit]\n\
         Description=freedesktop.org Secret Service backed by Windows Credential Manager\n\
         \n\
         [Service]\n\
         Type=dbus\n\
         BusName={BUS_NAME}\n\
         ExecStart={}\n\
         {env_line}\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display(),
    )
}

pub async fn install(dry: bool) -> Result<()> {
    if dry {
        println!("[dry-run] no changes will be made -- showing what `install` would do.\n");
    }

    let exe = std::env::current_exe().context("resolving own executable path")?;
    let exe = exe.canonicalize().unwrap_or(exe);

    let wincred_exe = std::env::var_os("WINCRED_EXE")
        .map(PathBuf::from)
        .or_else(|| find_on_path("wincred.exe"));
    match &wincred_exe {
        Some(p) => println!("found wincred.exe at {}", p.display()),
        None => eprintln!(
            "warning: couldn't find wincred.exe on PATH -- the installed service will likely \
             fail to find it either, since systemd user services get a minimal PATH that \
             doesn't include WSL's Windows interop entries. Re-run with WINCRED_EXE=<path> set, \
             or add `Environment=WINCRED_EXE=<path>` yourself via \
             `systemctl --user edit {UNIT_NAME}`."
        ),
    }

    let path = unit_dir()?.join(UNIT_NAME);
    let contents = unit_contents(&exe, wincred_exe.as_deref());
    write_file(dry, &path, &contents)?;

    disable_gnome_keyring_secrets(dry).await?;

    if !systemctl_apply(dry, &["enable", UNIT_NAME])? {
        bail!("systemctl --user enable {UNIT_NAME} failed");
    }

    if !dry {
        println!(
            "\n{UNIT_NAME} is set up and enabled. Restart WSL for it to take effect -- from \
             Windows, run `wsl --shutdown`, then open a new WSL window -- so it (and, if \
             applicable, the freed-up GNOME Keyring) start cleanly alongside everything else \
             that claims a place on the session bus at login. See README#install if it's \
             still not running after that."
        );
    }
    Ok(())
}

pub async fn uninstall(dry: bool) -> Result<()> {
    if dry {
        println!("[dry-run] no changes will be made -- showing what `uninstall` would do.\n");
    }

    let _ = systemctl_apply(dry, &["disable", UNIT_NAME]);
    let path = unit_dir()?.join(UNIT_NAME);
    if path.exists() {
        remove_file(dry, &path)?;
    } else {
        println!("{} not present, nothing to remove", path.display());
    }

    let dropin_dir = unit_dir()?.join("gnome-keyring-daemon.service.d");
    let dropin_path = dropin_dir.join("wincred-secretsd.conf");
    if dropin_path.exists() {
        remove_file(dry, &dropin_path)?;
        if !dry {
            let _ = std::fs::remove_dir(&dropin_dir); // only succeeds if now empty
        }
    }

    if !dry {
        println!(
            "\nRestart WSL (`wsl --shutdown` from Windows, then reopen) for this to fully take \
             effect -- wincred-secretsd keeps running until then, and GNOME Keyring's original \
             components aren't restored until it does too."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::without_secrets_component;

    #[test]
    fn drops_secrets_keeping_other_components_quoted() {
        let line = r#"/usr/bin/gnome-keyring-daemon --foreground --components="pkcs11,secrets" --control-directory=%t/keyring"#;
        assert_eq!(
            without_secrets_component(line).unwrap(),
            r#"/usr/bin/gnome-keyring-daemon --foreground --components="pkcs11" --control-directory=%t/keyring"#
        );
    }

    #[test]
    fn drops_secrets_from_middle_of_list() {
        let line = r#"gnome-keyring-daemon --components="pkcs11,secrets,ssh""#;
        assert_eq!(
            without_secrets_component(line).unwrap(),
            r#"gnome-keyring-daemon --components="pkcs11,ssh""#
        );
    }

    #[test]
    fn handles_unquoted_components() {
        let line = "gnome-keyring-daemon --components=pkcs11,secrets --foreground";
        assert_eq!(
            without_secrets_component(line).unwrap(),
            "gnome-keyring-daemon --components=pkcs11 --foreground"
        );
    }

    #[test]
    fn none_when_secrets_is_the_only_component() {
        let line = r#"gnome-keyring-daemon --components="secrets""#;
        assert!(without_secrets_component(line).is_none());
    }

    #[test]
    fn none_without_a_components_flag() {
        let line = "gnome-keyring-daemon --foreground";
        assert!(without_secrets_component(line).is_none());
    }
}
