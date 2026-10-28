//! Called only after the shared daemon-token check in the IPC router.
//!
//! An `AwaitProcess` start keeps its IPC stream open after `Opened` and
//! waits, without a time limit, for `BindProcess` on the same stream
//! (design 2.6, 9.3). A stream that ends before the binding closes the
//! tunnel. After the binding the tunnel lives as long as its QUIC connection
//! and the bound process: this service holds a process handle and closes
//! the tunnel when the process ends, even if its owner crashed.
use super::ipc_host::ShareHost;
use super::ipc_protocol::tcp::{TcpIpcReply, TcpIpcRequest};
use super::ipc_protocol::{write_response, IpcResponse};
use crate::share::auto_forward::SocketOwnerCheck;
use crate::share::tcp_forward::{ListenerAdmission, TunnelId};
use crate::share::tcp_registry::{BoundProcess, ListenerOptions, OriginCheck};
use crate::share::{route_cancellation::RouteCancellation, ShareService};
use std::{io, net::TcpStream, sync::Arc, time::Duration};

/// A `BindProcess` line is small; anything larger is not one.
const MAX_BIND_LINE: usize = 4096;

impl ShareHost {
    pub(in crate::daemon) fn serve_tcp(
        &self,
        mut stream: TcpStream,
        request: TcpIpcRequest,
    ) -> io::Result<()> {
        let awaits_process = matches!(
            &request,
            TcpIpcRequest::Start {
                admission: ListenerAdmission::AwaitProcess,
                ..
            }
        );
        let result = self.tcp_request(&mut stream, request);
        let opened = match &result {
            Ok(TcpIpcReply::Opened { tunnel }) => Some(tunnel.id.clone()),
            _ => None,
        };
        let response = match result {
            Ok(reply) => IpcResponse::Tcp(reply),
            Err(error) => IpcResponse::Err {
                msg: error,
                code: Some("tcp_forward_failed".into()),
            },
        };
        let written = write_response(&mut stream, &response);
        let (Some(id), Some(service)) = (opened, self.tcp_service()) else {
            return written;
        };
        if written.is_err() {
            service.tcp_close(&id);
            return written;
        }
        if !awaits_process {
            return written;
        }
        let bound = await_binding(&mut stream, &service, &id);
        let response = match bound {
            Ok(reply) => IpcResponse::Tcp(reply),
            Err(error) => {
                // No half state: an unbound or wrongly bound tunnel ends.
                service.tcp_close(&id);
                IpcResponse::Err {
                    msg: error,
                    code: Some("tcp_bind_failed".into()),
                }
            }
        };
        // A failed write after a successful binding leaves the tunnel to the
        // bound process, whose end closes it.
        write_response(&mut stream, &response)
    }
    fn tcp_service(&self) -> Option<ShareService> {
        self.state.lock().ok()?.service.clone()
    }

    fn tcp_request(
        &self,
        stream: &mut TcpStream,
        request: TcpIpcRequest,
    ) -> Result<TcpIpcReply, String> {
        match request {
            TcpIpcRequest::List => Ok(TcpIpcReply::Listed {
                tunnels: self
                    .tcp_service()
                    .map_or_else(Vec::new, |service| service.tcp_list()),
            }),
            TcpIpcRequest::Status { id } => Ok(TcpIpcReply::Status {
                tunnel: self
                    .tcp_service()
                    .and_then(|service| service.tcp_status(&id)),
            }),
            TcpIpcRequest::Close { id } => Ok(TcpIpcReply::Closed {
                closed: self
                    .tcp_service()
                    .is_some_and(|service| service.tcp_close(&id)),
            }),
            TcpIpcRequest::BindProcess { .. } => {
                Err("BindProcess gilt nur auf dem Strom eines Starts mit AwaitProcess".into())
            }
            TcpIpcRequest::Start {
                target,
                mut request,
                admission,
                bind,
            } => {
                admission_allowed(admission)?;
                // Version 2: the listener checks every connection's origin
                // (design 4.4); version-1 hosts still require the flag.
                request.acknowledge_local_access = true;
                request.validate()?;
                let check: Option<Arc<dyn OriginCheck>> = match admission {
                    ListenerAdmission::AnyLocal => None,
                    _ => Some(Arc::new(SocketOwnerCheck::new()?)),
                };
                let options = ListenerOptions {
                    admission,
                    bind,
                    check,
                };
                self.reload_now()?;
                let service = self.tcp_service().ok_or_else(|| {
                    "Share service is not configured or auto-connect is disabled".to_string()
                })?;
                let cancellation = Arc::new(RouteCancellation::default());
                let probe = stream.try_clone().map_err(|error| error.to_string())?;
                // Peek never consumes protocol bytes. A 1-ms bound keeps the
                // setup future responsive without a per-request OS thread.
                probe
                    .set_read_timeout(Some(Duration::from_millis(1)))
                    .map_err(|error| error.to_string())?;
                cancellation
                    .set_probe(Some(Arc::new(move || {
                        let mut byte = [0];
                        match probe.peek(&mut byte) {
                            Ok(_) => Ok(true), // EOF or unexpected client data.
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::WouldBlock
                                        | io::ErrorKind::TimedOut
                                        | io::ErrorKind::Interrupted
                                ) =>
                            {
                                Ok(false)
                            }
                            Err(error) => Err(error),
                        }
                    })))
                    .map_err(|error| error.to_string())?;
                let pending_cancel = cancellation.clone();
                let tunnel = service
                    .tcp_start_cancellable(
                        &target,
                        request,
                        options,
                        || {
                            if write_response(stream, &IpcResponse::Tcp(TcpIpcReply::Pending))
                                .is_err()
                            {
                                pending_cancel.cancel();
                            }
                        },
                        Some(cancellation),
                    )
                    .map_err(|error| error.to_string())?;
                Ok(TcpIpcReply::Opened { tunnel })
            }
        }
    }
}

/// Design 4.4: `AnyLocal` only where the OS discloses no connection owner
/// (Android, assumption A7); there every other rule would refuse every
/// connection, so it is refused up front with the reason.
fn admission_allowed(admission: ListenerAdmission) -> Result<(), String> {
    let android = cfg!(target_os = "android");
    match (admission, android) {
        (ListenerAdmission::AnyLocal, false) => Err(
            "Weiterleitungen für jeden lokalen Prozess gibt es nur auf Android; hier prüft se die Herkunft".into(),
        ),
        (ListenerAdmission::SameAccount | ListenerAdmission::AwaitProcess, true) => Err(
            "Android legt die Herkunft lokaler Verbindungen nicht offen; Weiterleitungen öffnen sich dort auf Tippen (AnyLocal)".into(),
        ),
        _ => Ok(()),
    }
}

/// Waits for the client's `BindProcess`, checks the process, holds its
/// handle and binds the tunnel. Every failure is returned; the caller then
/// closes the tunnel.
fn await_binding(
    stream: &mut TcpStream,
    service: &ShareService,
    id: &TunnelId,
) -> Result<TcpIpcReply, String> {
    // No time limit (design 2.6): the client sends the binding right after
    // it started its process, or its stream ends.
    stream
        .set_read_timeout(None)
        .map_err(|error| error.to_string())?;
    let mut line = String::new();
    super::line::read_line_limited_from_stream(stream, &mut line, MAX_BIND_LINE)
        .map_err(|error| format!("Prozessbindung nicht gelesen: {error}"))?;
    if line.trim().is_empty() {
        return Err("Der Aufrufer endete vor der Prozessbindung; Tunnel geschlossen".into());
    }
    let request: TcpIpcRequest = serde_json::from_str(line.trim())
        .map_err(|error| format!("Ungültige Prozessbindung: {error}"))?;
    let TcpIpcRequest::BindProcess {
        id: bound_id,
        pid,
        started,
    } = request
    else {
        return Err("Auf diesem Strom ist nur BindProcess erlaubt".into());
    };
    if &bound_id != id {
        return Err("Prozessbindung nennt einen anderen Tunnel".into());
    }
    let identity = crate::share::os_sockets::process_identity(pid)
        .map_err(|error| format!("Prozess {pid} nicht prüfbar: {error}"))?;
    if identity.started != started {
        return Err(format!(
            "Prozess {pid} wurde neu gestartet; Bindung abgelehnt"
        ));
    }
    let watched = service.clone();
    let watched_id = id.clone();
    crate::rdp::watch_process_exit(
        pid,
        started,
        Box::new(move || {
            watched.tcp_close(&watched_id);
        }),
    )?;
    service
        .tcp_bind_process(id, BoundProcess { pid, started })
        .map_err(|error| error.to_string())?;
    let tunnel = service
        .tcp_status(id)
        .ok_or_else(|| "Tunnel endete während der Prozessbindung".to_string())?;
    Ok(TcpIpcReply::Bound { tunnel })
}
