//! Called only after the shared daemon-token check in the IPC router.
use super::ipc_host::ShareHost;
use super::ipc_protocol::tcp::{TcpIpcReply, TcpIpcRequest};
use super::ipc_protocol::{write_response, IpcResponse};
use crate::share::{route_cancellation::RouteCancellation, ShareService};
use std::{io, net::TcpStream, sync::Arc, time::Duration};

impl ShareHost {
    pub(in crate::daemon) fn serve_tcp(
        &self,
        mut stream: TcpStream,
        request: TcpIpcRequest,
    ) -> io::Result<()> {
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
        if written.is_err() {
            if let (Some(id), Some(service)) = (opened, self.tcp_service()) {
                service.tcp_close(&id);
            }
        }
        written
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
            TcpIpcRequest::Start { target, request } => {
                request.validate()?;
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
