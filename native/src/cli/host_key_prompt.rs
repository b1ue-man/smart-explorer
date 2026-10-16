//! Interactive confirmation of an unknown SSH host key for `se` commands run
//! by a person: the command fails with the key's fingerprint, `se` asks on
//! the terminal (OpenSSH `StrictHostKeyChecking ask`), pins the confirmed key
//! and tries once more. Without a terminal on stdin and stderr (scripts, MCP,
//! pipes) the error stays, with the `se connections trust-host-key` command
//! that confirms a compared fingerprint.
use std::io::IsTerminal;
use std::sync::Arc;

use crate::creds::SavedConnection;
use crate::sftp::SftpBackend;

pub(crate) fn with_host_key_prompt<T>(
    mut open: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let error = match open() {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let Some(key) = crate::sftp::unknown_host_key(&error) else {
        return Err(error);
    };
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(error);
    }
    let presented = crate::sftp::PresentedHostKey {
        key: key.clone(),
        trusted: false,
        changed_in: None,
    };
    if !super::connections::ask_host_key(&presented)? {
        return Err(error);
    }
    crate::sftp::trust_host_key(&key.host, key.port, &key.fingerprint, false)
        .map_err(|error| format!("SSH host key of {}:{}: {error}", key.host, key.port))?;
    open()
}

/// `connect::open_saved_ssh` with the prompt (`se shell`, `se exec @label`).
pub(crate) fn open_saved_ssh(connection: &SavedConnection) -> Result<Arc<SftpBackend>, String> {
    with_host_key_prompt(|| crate::connect::open_saved_ssh(connection))
}
