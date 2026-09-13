use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Thin wrapper around invoking `wincred.exe` across the WSL/Windows
/// interop boundary. Every call spawns a fresh process (same cost a human
/// pays from the shell, ~60ms) -- fine for interactive Secret Service use;
/// if that ever becomes a bottleneck the fix is a `wincred.exe serve`
/// persistent-process mode, not a change here.
#[derive(Clone)]
pub struct Backend {
    exe: String,
}

#[derive(Deserialize)]
struct GetResp {
    secret: Option<String>,
}

#[derive(Deserialize)]
struct ListEntry {
    target: String,
}

impl Backend {
    pub fn new() -> Self {
        Self {
            exe: std::env::var("WINCRED_EXE").unwrap_or_else(|_| "wincred.exe".to_string()),
        }
    }

    pub async fn get(&self, target: &str) -> Result<Option<String>> {
        let out = Command::new(&self.exe)
            .args(["get", target, "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("spawning wincred.exe get")?;
        match out.status.code() {
            Some(0) => {
                let resp: GetResp = serde_json::from_slice(&out.stdout)
                    .context("parsing wincred get --json output")?;
                Ok(resp.secret)
            }
            Some(1) => Ok(None),
            _ => bail!(
                "wincred get {target} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        }
    }

    pub async fn set(&self, target: &str, username: &str, secret: &str) -> Result<()> {
        let mut child = Command::new(&self.exe)
            .args(["set", target, "--user", username])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawning wincred.exe set")?;
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(secret.as_bytes())
            .await
            .context("writing secret to wincred.exe set stdin")?;
        let out = child.wait_with_output().await?;
        if out.status.success() {
            Ok(())
        } else {
            bail!(
                "wincred set {target} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        }
    }

    pub async fn delete(&self, target: &str) -> Result<bool> {
        let out = Command::new(&self.exe)
            .args(["delete", target])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("spawning wincred.exe delete")?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!(
                "wincred delete {target} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        }
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let out = Command::new(&self.exe)
            .args(["list", "--prefix", prefix, "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("spawning wincred.exe list")?;
        if !out.status.success() {
            bail!(
                "wincred list {prefix} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let entries: Vec<ListEntry> =
            serde_json::from_slice(&out.stdout).context("parsing wincred list --json output")?;
        Ok(entries.into_iter().map(|e| e.target).collect())
    }
}
