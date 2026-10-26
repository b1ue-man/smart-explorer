//! Called only after the existing daemon-token gate in the IPC router.
use super::{
    ipc_host::ShareHost,
    ipc_protocol::{
        apps::{AppsIpcReply, AppsIpcRequest},
        write_response, IpcResponse,
    },
};
use crate::share::route_cancellation::RouteCancellation;
use std::{io, net::TcpStream, sync::Arc, time::Duration};
impl ShareHost {
    pub(in crate::daemon) fn serve_apps(
        &self,
        mut stream: TcpStream,
        request: AppsIpcRequest,
    ) -> io::Result<()> {
        let result = (|| -> Result<AppsIpcReply, String> {
            self.reload_now()?;
            let service = self
                .state
                .lock()
                .map_err(|_| "Share state unavailable")?
                .service
                .clone()
                .ok_or_else(|| {
                    "Share service is not configured or auto-connect is disabled".to_string()
                })?;
            let (target, command) = match request {
                AppsIpcRequest::LocalGrantSnapshot {
                    device_id,
                    public_key,
                } => {
                    return service
                        .local_app_grants(&device_id, &public_key)
                        .map(|snapshot| AppsIpcReply::LocalGrantSnapshot { snapshot })
                        .map_err(|e| e.to_string())
                }
                AppsIpcRequest::SetLocalGrant { change } => {
                    let patch = service
                        .prepare_local_app_grant(&change)
                        .map_err(|e| e.to_string())?;
                    let (result, _) = self.mutate_access_grant(
                        change.target,
                        &patch,
                        Some(change.expected_policy_revision),
                    )?;
                    return Ok(AppsIpcReply::LocalGrantMutation { result });
                }
                AppsIpcRequest::Command { target, command } => (target, command),
            };
            let cancellation = Arc::new(RouteCancellation::default());
            let probe = stream.try_clone().map_err(|e| e.to_string())?;
            probe
                .set_read_timeout(Some(Duration::from_millis(1)))
                .map_err(|e| e.to_string())?;
            cancellation
                .set_probe(Some(Arc::new(move || {
                    let mut byte = [0];
                    match probe.peek(&mut byte) {
                        Ok(_) => Ok(true),
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock
                                    | io::ErrorKind::TimedOut
                                    | io::ErrorKind::Interrupted
                            ) =>
                        {
                            Ok(false)
                        }
                        Err(e) => Err(e),
                    }
                })))
                .map_err(|e| e.to_string())?;
            let cancel = cancellation.clone();
            let reply = service
                .apps_command(
                    &target,
                    command,
                    || {
                        if write_response(&mut stream, &IpcResponse::Apps(AppsIpcReply::Pending))
                            .is_err()
                        {
                            cancel.cancel();
                        }
                    },
                    Some(cancellation),
                )
                .map_err(|e| e.to_string())?;
            Ok(AppsIpcReply::Completed {
                node_id: reply.node_id,
                device_id: reply.device_id,
                result: reply.result,
            })
        })();
        let response = match result {
            Ok(reply) => IpcResponse::Apps(reply),
            Err(msg) => IpcResponse::Err {
                msg,
                code: Some("device_apps_failed".into()),
            },
        };
        // A lost response does not stop the host app or invent a retry. The
        // original request ID recovers its peer-owned durable receipt.
        if serde_json::to_vec(&response)
            .map_err(io::Error::other)?
            .len()
            .saturating_add(1)
            > crate::daemon::line::MAX_IPC_LINE
        {
            return write_response(&mut stream,&IpcResponse::Err {msg:"Complete device-command result exceeds daemon frame; no partial inventory returned".into(),code:Some("device_inventory_too_large".into())});
        }
        write_response(&mut stream, &response)
    }
}
