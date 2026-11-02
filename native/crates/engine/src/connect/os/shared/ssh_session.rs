//! Authenticated SSH session of a saved SFTP connection for user commands
//! (`se exec @label`, the MCP `exec` tool). Only the SFTP login is reused:
//! the acceleration agent is never deployed and nothing is persisted.
use crate::creds::{Protocol, SavedConnection};
use crate::sftp::SftpBackend;
use std::sync::Arc;

pub fn open_saved_ssh(connection: &SavedConnection) -> Result<Arc<SftpBackend>, String> {
    if connection.protocol != Protocol::Sftp {
        return Err(format!(
            "{} is a {} connection; remote commands need an SFTP (SSH) connection or a Share peer",
            connection.display(),
            connection.protocol.as_str()
        ));
    }
    let secret = crate::creds::get_secret_checked(&connection.account())
        .map_err(|error| format!("saved credential read failed: {error}"))?;
    let mut form = crate::connect::ConnectForm::from_saved(connection);
    form.save = false;
    form.use_agent = false;
    match super::connector::do_connect_with_agent_fallback(
        form,
        secret,
        super::connector::AgentFallback::Allow,
    ) {
        crate::connect::ConnectResult::Ok(connected) => connected
            .remote
            .and_then(|remote| remote.sftp)
            .ok_or_else(|| "the SFTP login did not provide an SSH session".to_string()),
        crate::connect::ConnectResult::Err(error) => Err(error),
    }
}
