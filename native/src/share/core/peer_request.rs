use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use crate::vfs::VfsResult;

use super::backend::PeerBackend;
use super::core::eio;
use super::framing::{decode_resp, recv_resp_wire, send_ctrl, TAG_DATA};
use super::io_deadline;
use super::node_sessions::OpenedPeerStream;
use super::wire::{Ctrl, FsRequest, FsResponse};

pub(super) const CONTROL_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
const IDEMPOTENT_CONTROL_BUDGET: Duration = Duration::from_secs(40);

impl PeerBackend {
    pub(super) fn request(&self, req: FsRequest) -> io::Result<FsResponse> {
        let lease = self.mount_lease.current()?;
        let budget = if is_retryable_read(&req) {
            IDEMPOTENT_CONTROL_BUDGET
        } else {
            io_deadline::PEER_OP_TIMEOUT
        };
        self.request_with_lease_until(req, lease, Instant::now() + budget)
    }

    pub(super) fn request_unleased_until(
        &self,
        req: FsRequest,
        deadline: Instant,
    ) -> io::Result<FsResponse> {
        self.request_with_lease_until(req, None, deadline)
    }

    fn request_with_lease_until(
        &self,
        req: FsRequest,
        lease: Option<String>,
        deadline: Instant,
    ) -> io::Result<FsResponse> {
        let retryable = is_retryable_read(&req);
        let max_attempts = if retryable { 2 } else { 1 };
        let operation = super::peer_fs_logging::request_label(&req);
        let started = Instant::now();
        let mut last_error = None;
        for attempt in 0..max_attempts {
            let endpoint = self.current_endpoint()?;
            let connect_deadline = control_attempt_deadline(deadline)?;
            let opened =
                match self
                    .node
                    .open_stream_until(&endpoint, &self.identity, connect_deadline)
                {
                    Ok(opened) => opened,
                    Err(error) => {
                        last_error = Some(error);
                        if attempt + 1 < max_attempts {
                            continue;
                        }
                        break;
                    }
                };
            let response_deadline = if retryable {
                connect_deadline
            } else {
                Instant::now() + io_deadline::PEER_OP_TIMEOUT
            };
            match self.request_once(opened, req.clone(), lease.clone(), response_deadline) {
                Ok(response) => {
                    let response = decode_resp(response)?;
                    super::peer_telemetry::report_fs_success(
                        &self.node.ev,
                        operation,
                        started,
                        &response,
                    );
                    return Ok(response);
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 >= max_attempts {
                        break;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| eio("Peer-Anfrage ohne Ergebnis beendet")))
    }

    fn request_once(
        &self,
        mut opened: OpenedPeerStream,
        req: FsRequest,
        lease: Option<String>,
        deadline: Instant,
    ) -> io::Result<FsResponse> {
        let expected = req.clone();
        let timeout = match io_deadline::remaining(deadline, "peer filesystem request") {
            Ok(timeout) => timeout,
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                return Err(error);
            }
        };
        let result = self.node.block_on(io_deadline::run_for(
            "peer filesystem request",
            timeout,
            async {
                send_ctrl(&mut opened.send, &Ctrl::Fs { req, lease }).await?;
                recv_resp_wire(&mut opened.recv).await
            },
        ));
        let result = result.and_then(|response| {
            response_matches(&expected, &response)
                .then_some(response)
                .ok_or_else(|| eio("Peer sendet eine unpassende Dateisystem-Antwort"))
        });
        if result.is_err() {
            io_deadline::abort(&mut opened.send, &mut opened.recv);
            let _ = self
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
        result
    }

    pub(super) fn open_reader(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let deadline = Instant::now() + IDEMPOTENT_CONTROL_BUDGET;
        let lease = self.mount_lease.current()?;
        let mut last_error = None;
        for attempt in 0..2 {
            let endpoint = self.current_endpoint()?;
            let attempt_deadline = control_attempt_deadline(deadline)?;
            let mut opened =
                match self
                    .node
                    .open_stream_until(&endpoint, &self.identity, attempt_deadline)
                {
                    Ok(opened) => opened,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
            let timeout = match io_deadline::remaining(attempt_deadline, "peer read open") {
                Ok(timeout) => timeout,
                Err(error) => {
                    io_deadline::abort(&mut opened.send, &mut opened.recv);
                    let _ = self
                        .node
                        .invalidate_outgoing_session(&opened.session_key, opened.generation);
                    last_error = Some(error);
                    continue;
                }
            };
            let result =
                self.node
                    .block_on(io_deadline::run_for("peer read open", timeout, async {
                        send_ctrl(
                            &mut opened.send,
                            &Ctrl::Fs {
                                req: FsRequest::Read {
                                    path: path.to_string(),
                                },
                                lease: lease.clone(),
                            },
                        )
                        .await?;
                        recv_resp_wire(&mut opened.recv).await
                    }));
            match result {
                Ok(response) => match decode_resp(response) {
                    Ok(FsResponse::Data { size }) => {
                        return Ok(super::peer_read::reader(
                            self.node.clone(),
                            opened.recv,
                            size,
                            TAG_DATA,
                            opened.session_key,
                            opened.generation,
                        ));
                    }
                    Ok(_) => {
                        io_deadline::abort(&mut opened.send, &mut opened.recv);
                        let _ = self
                            .node
                            .invalidate_outgoing_session(&opened.session_key, opened.generation);
                        return Err(eio("unerwartete Antwort auf read"));
                    }
                    Err(error) => return Err(error),
                },
                Err(error) => {
                    io_deadline::abort(&mut opened.send, &mut opened.recv);
                    let _ = self
                        .node
                        .invalidate_outgoing_session(&opened.session_key, opened.generation);
                    last_error = Some(error);
                    if attempt == 1 {
                        break;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| eio("Peer-Leseanforderung ohne Ergebnis beendet")))
    }

    pub(super) fn open_writer(
        &self,
        request: FsRequest,
        operation: &'static str,
    ) -> VfsResult<Box<dyn Write + Send>> {
        let lease = self.mount_lease.current()?;
        let endpoint = self.current_endpoint()?;
        let deadline = Instant::now() + io_deadline::PEER_OP_TIMEOUT;
        let connect_deadline = control_attempt_deadline(deadline)?;
        let mut opened =
            self.node
                .open_stream_until(&endpoint, &self.identity, connect_deadline)?;
        let response_deadline = Instant::now() + io_deadline::PEER_OP_TIMEOUT;
        let timeout = match io_deadline::remaining(response_deadline, operation) {
            Ok(timeout) => timeout,
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                return Err(error);
            }
        };
        let result = self
            .node
            .block_on(io_deadline::run_for(operation, timeout, async {
                send_ctrl(
                    &mut opened.send,
                    &Ctrl::Fs {
                        req: request,
                        lease: lease.clone(),
                    },
                )
                .await?;
                recv_resp_wire(&mut opened.recv).await
            }));
        let response = match result {
            Ok(response) => match decode_resp(response) {
                Ok(response) => response,
                Err(error) => return Err(error),
            },
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                return Err(error);
            }
        };
        if !matches!(response, FsResponse::Ready) {
            io_deadline::abort(&mut opened.send, &mut opened.recv);
            let _ = self
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
            return Err(eio("unerwartete Antwort auf write"));
        }
        Ok(super::peer_writer::writer(
            self.node.clone(),
            opened.send,
            opened.recv,
            lease,
            opened.session_key,
            opened.generation,
        ))
    }
}

fn control_attempt_deadline(overall: Instant) -> io::Result<Instant> {
    io_deadline::remaining(overall, "peer control operation")?;
    Ok(overall.min(Instant::now() + CONTROL_ATTEMPT_TIMEOUT))
}

fn is_retryable_read(request: &FsRequest) -> bool {
    matches!(
        request,
        FsRequest::Capabilities { .. } | FsRequest::ListDir { .. } | FsRequest::Stat { .. }
    )
}

fn response_matches(request: &FsRequest, response: &FsResponse) -> bool {
    if matches!(response, FsResponse::Err { .. }) {
        return true;
    }
    match request {
        FsRequest::Capabilities { .. } => matches!(response, FsResponse::Capabilities { .. }),
        FsRequest::ListDir { .. } => matches!(response, FsResponse::Entries { .. }),
        FsRequest::Stat { .. } => matches!(response, FsResponse::Meta { .. }),
        FsRequest::CopyFile { .. } => {
            matches!(response, FsResponse::Data { .. } | FsResponse::Ok)
        }
        FsRequest::Rename { .. }
        | FsRequest::RenameNoReplace { .. }
        | FsRequest::PromoteStaged { .. }
        | FsRequest::RemoveFile { .. }
        | FsRequest::RemoveDir { .. }
        | FsRequest::MkdirAll { .. }
        | FsRequest::ReleaseLease => matches!(response, FsResponse::Ok),
        FsRequest::Read { .. }
        | FsRequest::Write { .. }
        | FsRequest::WriteNew { .. }
        | FsRequest::WriteDone
        | FsRequest::WalkTree { .. }
        | FsRequest::StorageSnapshot { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_only_idempotent_control_reads_are_replayable() {
        assert!(is_retryable_read(&FsRequest::ListDir { path: "/".into() }));
        assert!(is_retryable_read(&FsRequest::Stat { path: "/".into() }));
        assert!(is_retryable_read(&FsRequest::Capabilities {
            path: "/Docs".into(),
            acquire_lease: true,
            lease_request_id: Some("mount-request".into()),
        }));
        assert!(!is_retryable_read(&FsRequest::Read { path: "/x".into() }));
        assert!(!is_retryable_read(&FsRequest::Write { path: "/x".into() }));
        assert!(!is_retryable_read(&FsRequest::Rename {
            src: "/a".into(),
            dst: "/b".into(),
        }));
        assert!(!is_retryable_read(&FsRequest::RemoveFile {
            path: "/x".into()
        }));
        assert!(!is_retryable_read(&FsRequest::ReleaseLease));
    }
}
