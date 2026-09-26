//! Atomic replace for plain SFTP. SFTP v3 `SSH_FXP_RENAME` never replaces an
//! existing file; OpenSSH and compatible servers offer
//! `posix-rename@openssh.com`, which is one `rename(2)` on the server. The
//! main session (`SftpSession`) does not expose protocol extensions, so the
//! request runs on a short-lived second SFTP subsystem channel of the same SSH
//! connection. Servers without the extension keep the explicit refusal.
use std::io;
use std::time::Duration;

use russh::client::Msg;
use russh::Channel;
use russh_sftp::client::RawSftpSession;
use russh_sftp::extensions::HardlinkExtension;
use russh_sftp::protocol::{Packet, StatusCode};

use super::io_err;

const POSIX_RENAME: &str = "posix-rename@openssh.com";
const SUBSYSTEM_DEADLINE: Duration = Duration::from_secs(10);
/// Per-request limit of the raw session (init, rename).
const REQUEST_TIMEOUT_SECS: u64 = 20;

/// Renames `from` over `to` on `channel` (a fresh session channel), replacing
/// an existing `to` atomically. `Unsupported` when the server lacks the
/// extension; nothing is changed then.
pub(super) async fn posix_rename(channel: Channel<Msg>, from: &str, to: &str) -> io::Result<()> {
    tokio::time::timeout(SUBSYSTEM_DEADLINE, channel.request_subsystem(true, "sftp"))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SFTP subsystem request timed out"))?
        .map_err(io_err)?;
    let session = RawSftpSession::new(channel.into_stream());
    session.set_timeout(REQUEST_TIMEOUT_SECS);
    let result = rename_on(&session, from, to).await;
    let _ = session.close_session();
    result
}

async fn rename_on(session: &RawSftpSession, from: &str, to: &str) -> io::Result<()> {
    let version = session.init().await.map_err(io_err)?;
    if version.extensions.get(POSIX_RENAME).map(String::as_str) != Some("1") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Der SFTP-Server bietet kein atomares Ersetzen (posix-rename@openssh.com); \
             vorhandene Dateien lassen sich hier nur mit Remote-Agent ersetzen",
        ));
    }
    // posix-rename carries the same payload as hardlink@openssh.com: two strings.
    let payload: Vec<u8> = HardlinkExtension {
        oldpath: from.to_string(),
        newpath: to.to_string(),
    }
    .try_into()
    .map_err(|error| io::Error::other(format!("posix-rename payload: {error}")))?;
    match session
        .extended(POSIX_RENAME, payload)
        .await
        .map_err(io_err)?
    {
        Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
        Packet::Status(status) => Err(io::Error::other(format!(
            "posix-rename {from} → {to}: {}: {}",
            status.status_code, status.error_message
        ))),
        _ => Err(io::Error::other(
            "posix-rename: unerwartete Antwort des SFTP-Servers",
        )),
    }
}
