//! Client side of ranged transfers on the Share (`ranges_v1`, `lz4_v1`): what
//! the host advertised, a range read as one request, and the stage that
//! several requests fill range by range (`SegmentedStage`).
//!
//! The host picks the stage's name and keeps its handle in the table of the
//! connection; this client only names the file the stage belongs to. A host
//! without `ranges_v1` gets none of these requests.
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Instant;

use super::peer_stages::StageTicket;
use super::PeerBackend;
use crate::share::core::eio;
use crate::share::framing::lz4_stream::{frame_for, Lz4Stream};
use crate::share::framing::{decode_resp, recv_resp_wire, send_ctrl, send_tagged};
use crate::share::identity::ShareIdentity;
use crate::share::io_deadline;
use crate::share::node::ShareIrohNode;
use crate::share::peer_request::CONTROL_ATTEMPT_TIMEOUT;
use crate::share::quic_stats_hub::track_client;
use crate::share::transfer_meter as stats;
use crate::share::types::PeerEndpoint;
use crate::share::wire::{
    Ctrl, FsRequest, FsResponse, FsSegmentedFinish, FsSegmentedOpen, FsStageWrite,
};
use crate::vfs::{SegmentedStage, VfsResult};

const STAGE_MARKER: &str = ".se-upload-";

/// The destination the engine's stage name `hint` belongs to
/// (`<file>.se-upload-<16 hex>`), if it has that shape.
fn destination_of(hint: &str) -> Option<&str> {
    let position = hint.rfind(STAGE_MARKER)?;
    let suffix = &hint[position + STAGE_MARKER.len()..];
    let hex = suffix.len() == 16
        && suffix
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    (position > 0 && hex).then(|| &hint[..position])
}

impl PeerBackend {
    /// Whether this client asks for LZ4 content: the host advertised
    /// `lz4_v1` in its capabilities (authenticated by the session). Nothing
    /// is packed in either direction otherwise.
    pub(in crate::share) fn lz4_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.lz4_v1 && !crate::agent_proto::lz4_switched_off()
    }

    /// Whether the host reads and writes ranges (`ranges_v1`).
    pub(in crate::share) fn ranges_offered(&self) -> bool {
        let caps = self.transfer_caps();
        caps.v1 && caps.ranges_v1
    }

    /// Reads `[offset, offset + len)` (fewer only at the end of the file) as
    /// one request; `None` where the host does not advertise `ranges_v1`. A
    /// stream that ends before the announced length is an error of the
    /// reader, never a complete range.
    pub(in crate::share) fn read_range(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
        len: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if len == 0 {
            // On the wire 0 means "to the end".
            return Ok(Some(Box::new(io::empty())));
        }
        if !self.ranges_offered() {
            return Ok(None);
        }
        let request = FsRequest::ReadAt {
            path: path.to_string(),
            id: id.map(str::to_string),
            offset,
            len,
            lz4: self.lz4_offered(),
        };
        match self.open_read_request(request, "peer read range") {
            Ok(reader) => Ok(Some(reader)),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A stage for `size` bytes next to the destination of the engine stage
    /// `stage_hint`; the host picks the name. `None` unless the host
    /// advertised `ranges_v1` and can write ranges below this path.
    pub(super) fn open_ranged_stage(
        &self,
        stage_hint: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        if !self.ranges_offered() {
            return Ok(None);
        }
        let Some(destination) = destination_of(stage_hint) else {
            return Ok(None);
        };
        let request = FsRequest::SegmentedStageOpen(FsSegmentedOpen {
            path: destination.to_string(),
            size,
        });
        let response = match self.request(request) {
            Ok(response) => response,
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
            Err(error) => return Err(error),
        };
        let FsResponse::SegmentedStageReady { stage } = response else {
            return Err(eio("unerwartete Antwort auf segmented_stage_open"));
        };
        let ticket = self.reserve_stage(&stage)?;
        if let Some(ticket) = &ticket {
            ticket.opened()?;
        }
        Ok(Some(Box::new(PeerSegmentedStage {
            node: self.node.clone(),
            identity: self.identity.clone(),
            endpoint: self.current_endpoint()?,
            lease: self.mount_lease_token()?,
            stage,
            lz4: self.lz4_offered(),
            ticket,
            size,
        })))
    }
}

/// The stage as the client sees it: its path, and what a range request needs
/// to open a stream of its own. The stage lives in the table of one host
/// connection; if that connection is replaced, the host has dropped it and
/// the next request fails.
struct PeerSegmentedStage {
    node: Arc<ShareIrohNode>,
    identity: ShareIdentity,
    endpoint: PeerEndpoint,
    lease: Option<String>,
    /// The name the host picked.
    stage: String,
    lz4: bool,
    /// Tracks this client's ownership until the engine publishes the stage.
    ticket: Option<StageTicket>,
    size: u64,
}

impl PeerSegmentedStage {
    /// One request on a stream of its own, answered with one reply.
    fn call(&self, request: FsRequest) -> io::Result<FsResponse> {
        let mut opened = self.node.open_stream_until(
            &self.endpoint,
            &self.identity,
            Instant::now() + CONTROL_ATTEMPT_TIMEOUT,
        )?;
        let reply = self
            .node
            .block_on(io_deadline::run("peer stage request", async {
                send_ctrl(
                    &mut opened.send,
                    &Ctrl::Fs {
                        req: request,
                        lease: self.lease.clone(),
                    },
                )
                .await?;
                recv_resp_wire(&mut opened.recv).await
            }));
        match reply.and_then(decode_resp) {
            Ok(response) => Ok(response),
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                if !matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                ) {
                    let _ = self
                        .node
                        .invalidate_outgoing_session(&opened.session_key, opened.generation);
                }
                Err(error)
            }
        }
    }
}

impl SegmentedStage for PeerSegmentedStage {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    /// A new stream: `StageWriteAt`, then exactly `len` bytes, then the end
    /// of the stream; `flush` waits for the host's `Ok`.
    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        let mut opened = self.node.open_stream_until(
            &self.endpoint,
            &self.identity,
            Instant::now() + CONTROL_ATTEMPT_TIMEOUT,
        )?;
        let request = FsRequest::StageWriteAt(FsStageWrite {
            stage: self.stage.clone(),
            offset,
            len,
            lz4: self.lz4,
        });
        let lease = self.lease.clone();
        let answered = self
            .node
            .block_on(io_deadline::run("peer stage range open", async {
                send_ctrl(
                    &mut opened.send,
                    &Ctrl::Fs {
                        req: request,
                        lease,
                    },
                )
                .await?;
                recv_resp_wire(&mut opened.recv).await
            }));
        match answered.and_then(decode_resp) {
            Ok(FsResponse::Ready) => {}
            Ok(_) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                return Err(eio("unerwartete Antwort auf stage_write_at"));
            }
            Err(error) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                return Err(error);
            }
        }
        track_client(&self.node, &opened.session_key);
        stats::stream_started();
        Ok(Box::new(RangeWriter {
            node: self.node.clone(),
            opened: Some(opened),
            expected: len,
            sent: 0,
            lz4: self.lz4.then(Lz4Stream::new),
            state: RangeState::Open,
        }))
    }

    /// The host checks the coverage and the length; the stage then waits for
    /// the engine's publication.
    fn finish(self: Box<Self>) -> VfsResult<()> {
        let request = FsRequest::SegmentedStageFinish(FsSegmentedFinish {
            stage: self.stage.clone(),
        });
        match self.call(request)? {
            FsResponse::Ok => {}
            _ => return Err(eio("unerwartete Antwort auf segmented_stage_finish")),
        }
        if let Some(ticket) = &self.ticket {
            ticket.committed(self.size)?;
        }
        Ok(())
    }

    /// The host removes the stage and its table entry; if the request does
    /// not arrive, it removes the stage when the connection ends.
    fn abort(self: Box<Self>) {
        let request = FsRequest::DiscardStage {
            path: self.stage.clone(),
        };
        let removed = matches!(self.call(request), Ok(FsResponse::Ok));
        if let Some(ticket) = &self.ticket {
            if removed {
                ticket.release();
            } else {
                ticket.uncertain();
            }
        }
    }
}

enum RangeState {
    Open,
    Committed,
    Failed(io::ErrorKind, String),
}

/// The writer of one range: it sends exactly `expected` bytes, ends its
/// stream and commits when the host answers `Ok`.
struct RangeWriter {
    node: Arc<ShareIrohNode>,
    opened: Option<crate::share::node_sessions::OpenedPeerStream>,
    expected: u64,
    sent: u64,
    lz4: Option<Lz4Stream>,
    state: RangeState,
}

impl RangeWriter {
    fn fail(&mut self, error: io::Error) -> io::Error {
        if let Some(mut opened) = self.opened.take() {
            io_deadline::abort(&mut opened.send, &mut opened.recv);
            let _ = self
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
        self.state = RangeState::Failed(error.kind(), error.to_string());
        error
    }

    fn finish(&mut self) -> io::Result<()> {
        match &self.state {
            RangeState::Committed => return Ok(()),
            RangeState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            RangeState::Open => {}
        }
        if self.sent != self.expected {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} statt {} Bytes für den Bereich",
                    self.sent, self.expected
                ),
            );
            return Err(self.fail(error));
        }
        let Some(mut opened) = self.opened.take() else {
            return Err(eio("Bereichsstrom geschlossen"));
        };
        let result = self
            .node
            .block_on(io_deadline::run("peer stage range end", async {
                opened.send.finish().map_err(eio)?;
                recv_resp_wire(&mut opened.recv).await
            }));
        match result.and_then(decode_resp) {
            Ok(FsResponse::Ok) => {
                self.state = RangeState::Committed;
                Ok(())
            }
            Ok(_) => {
                self.opened = Some(opened);
                Err(self.fail(eio("unerwartete Antwort auf das Bereichsende")))
            }
            Err(error) => {
                self.opened = Some(opened);
                Err(self.fail(error))
            }
        }
    }
}

impl Write for RangeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &self.state {
            RangeState::Committed => {
                return Err(eio("Bereichsstrom ist bereits abgeschlossen"));
            }
            RangeState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()));
            }
            RangeState::Open => {}
        }
        let total = self.sent.saturating_add(buf.len() as u64);
        if total > self.expected {
            let error = io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Quelle liefert mehr als die {} Bytes des Bereichs",
                    self.expected
                ),
            );
            return Err(self.fail(error));
        }
        for chunk in buf.chunks(crate::share::fs::CHUNK) {
            let (tag, bytes) = frame_for(self.lz4.as_ref(), chunk);
            let started = Instant::now();
            let Some(opened) = self.opened.as_mut() else {
                return Err(eio("Bereichsstrom geschlossen"));
            };
            let result = self.node.block_on(io_deadline::run(
                "peer stage range data",
                send_tagged(&mut opened.send, tag, &bytes),
            ));
            if stats::enabled() {
                stats::meter().add(stats::Phase::NetWait, started.elapsed());
            }
            if let Err(error) = result {
                return Err(self.fail(error));
            }
            if let Some(lz4) = &self.lz4 {
                lz4.record_send(chunk.len(), started.elapsed(), Instant::now());
            }
        }
        self.sent = total;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for RangeWriter {
    fn drop(&mut self) {
        if matches!(self.state, RangeState::Open) {
            // Dropping the streams resets them: the host frees the range and
            // writes nothing more. Only `flush` commits a range.
            drop(self.opened.take());
        }
    }
}
