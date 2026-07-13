use std::net::TcpStream;
use std::time::Duration;

use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};

pub(crate) fn mutate_exec_grant(
    target: crate::share::ExecGrantTarget,
    enabled: bool,
) -> Result<super::super::ipc_host::exec_grant_journal::ExecGrantPersistResult, String> {
    super::ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = read_ipc_addr().ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_secs(15)));
    write_request(
        &mut stream,
        &IpcRequest::MutateExecGrant {
            token,
            target,
            enabled,
        },
    )
    .map_err(|error| error.to_string())?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::ExecGrantMutation { result } => Ok(result),
        IpcResponse::Err { msg } => Err(msg),
        _ => Err("Unerwartete Worker-Antwort".into()),
    }
}
