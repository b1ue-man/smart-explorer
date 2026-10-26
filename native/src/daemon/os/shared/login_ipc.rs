//! Only the daemon-token router may enter this login session facade.
use std::{io, net::TcpStream};
use super::{ipc_host::ShareHost, ipc_protocol::{write_response, IpcResponse, login::{LoginIpcRequest, LoginIpcReply}}};
#[path = "login_ipc_registry.rs"]
mod registry;
#[path = "login_ipc_session.rs"]
mod session;
impl ShareHost {
    pub(in crate::daemon) fn serve_login(&self, mut stream: TcpStream, request: LoginIpcRequest) -> io::Result<()> {
        let result = match request {
            LoginIpcRequest::Status => Ok(LoginIpcReply::Status { installation: crate::login::installation_status() }),
            LoginIpcRequest::Sessions => registry::sessions().map(|sessions| LoginIpcReply::Sessions { sessions }),
            LoginIpcRequest::Session { session } => registry::session(&session).map(|session| LoginIpcReply::Session { session }),
            LoginIpcRequest::Stop { session } => registry::stop(session.as_deref()).map(|count| LoginIpcReply::Stopped { count }),
            LoginIpcRequest::Stream { target, target_user, acknowledge_machine_pin } => {
                let context = (|| {
                    if !acknowledge_machine_pin { return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                        "login needs explicit pinning of the administrator-provided public machine target")); }
                    target.validate()?;
                    if target_user.is_empty() || target_user.len() > 256 { return Err(io::Error::other("invalid login target user")); }
                    self.reload_now().map_err(io::Error::other)?;
                    let service = self.state.lock().map_err(|_| io::Error::other("Share state is unavailable"))?.service.clone()
                        .ok_or_else(|| io::Error::other("Share service is not configured or auto-connect is disabled"))?;
                    service.login_client_context()
                })();
                return match context {
                    Ok(context) => session::serve(stream, context, target, target_user),
                    Err(error) => write_response(&mut stream, &failure(error)),
                };
            }
        };
        let response = match result { Ok(reply) => IpcResponse::Login(reply), Err(error) => failure(error) };
        if serde_json::to_vec(&response).map_err(io::Error::other)?.len().saturating_add(1) > crate::daemon::line::MAX_IPC_LINE {
            return write_response(&mut stream, &IpcResponse::Err { msg: "complete login status exceeds the daemon JSON limit; no partial inventory returned".into(),
                code: Some("login_inventory_too_large".into()) });
        }
        write_response(&mut stream, &response)
    }
}
fn failure(error: io::Error) -> IpcResponse {
    IpcResponse::Err { msg: error.to_string(), code: Some("login_unavailable".into()) }
}
/// Main calls this with the daemon's normal stop path before its runtime exits.
pub(crate) fn shutdown_login_viewers() -> io::Result<usize> { registry::drain() }
