//! Dedicated tokens live in Secret Service; existing Git helpers are read-only.
use super::{Account, CredentialSource};
use crate::core::operations;
use anyhow::{Context, Result, bail, ensure};
use std::{
    fmt,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> Result<Self> {
        ensure!(!value.is_empty() && !value.chars().any(char::is_control), "Enter a valid token");
        Ok(Self(value))
    }
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
fn tool(account: &Account, verb: &str) -> Command {
    let mut command = Command::new("secret-tool");
    command.arg(verb);
    if verb == "store" {
        command.arg("--label=Git hosting account");
    }
    command.args(["application", "nyx-git", "account", &account.id, "instance", &account.base_url]);
    command.stderr(Stdio::null());
    command
}
pub fn save(account: &Account, token: &Secret) -> Result<()> {
    account.base()?;
    let mut child = tool(account, "store")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("Install libsecret and enable a Secret Service keyring to save tokens")?;
    let mut input = child.stdin.take().context("Could not open the keyring input")?;
    let result = input.write_all(token.expose().as_bytes());
    drop(input);
    let status = child.wait()?;
    result.context("Could not send token to Secret Service")?;
    ensure!(status.success(), "Secret Service could not save the token. Unlock the desktop keyring; no account was added.");
    Ok(())
}
pub fn delete(account: &Account) -> Result<()> {
    if account.source == CredentialSource::SecretService {
        let result = tool(account, "clear")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .context("Could not reach Secret Service")?;
        ensure!(result.success(), "The saved token could not be removed; unlock the desktop keyring and retry");
    }
    Ok(())
}
pub fn load(account: &Account) -> Result<Secret> {
    let base = account.base()?;
    match account.source {
        CredentialSource::SecretService => {
            let result = tool(account, "lookup")
                .stdin(Stdio::null())
                .output()
                .context("Install libsecret and unlock the desktop keyring")?;
            ensure!(result.status.success(), "Token unavailable. Unlock the desktop keyring or reconnect this account.");
            let value = String::from_utf8(result.stdout).map_err(|_| anyhow::anyhow!("Invalid keyring credential"))?;
            Secret::new(value.trim_end_matches('\n').into())
        }
        CredentialSource::GitHelper => {
            let host = match base.port() {
                Some(port) => format!("{}:{port}", base.host_str().unwrap()),
                None => base.host_str().unwrap().into(),
            };
            ensure!(!account.credential_username.chars().any(char::is_control), "Invalid account username");
            let input = format!(
                "protocol=https\nhost={host}\npath={}\n{}\n",
                base.path().trim_start_matches('/'),
                if account.credential_username.is_empty() {
                    String::new()
                } else {
                    format!("username={}\n", account.credential_username)
                }
            );
            let output = operations::invoke(
                Path::new("/"),
                "Read Git credential",
                vec!["credential".into(), "fill".into()],
                Some(input.into_bytes()),
                &[],
                false,
            )
            .map_err(|_| anyhow::anyhow!("No usable Git helper credential. Connect with an API token instead."))?;
            let text = String::from_utf8(output).map_err(|_| anyhow::anyhow!("Invalid Git helper credential"))?;
            for line in text.lines() {
                if let Some(password) = line.strip_prefix("password=") {
                    return Secret::new(password.into());
                }
            }
            bail!("The Git credential helper did not return a password/token")
        }
    }
}
