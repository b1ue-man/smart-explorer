use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use iroh::endpoint::{RecvStream, SendStream};

use super::backend::peer_evidence::WrittenReport;
use super::backend::peer_stages::StageTicket;
use super::core::eio;
use super::framing::lz4_stream::{frame_for, Lz4Stream};
use super::framing::{decode_resp, recv_resp_wire, send_ctrl, send_tagged};
use super::fs;
use super::node::ShareIrohNode;
use super::quic_stats_hub::track_client;
use super::transfer_meter as stats;
use super::wire::{Ctrl, FsRequest, FsResponse, FsWritten};

/// A successful WriteDone acknowledgement, rather than Ready or the filename,
/// makes an exclusively created stage available to finish/publication.
pub(super) fn owned_writer(
    inner: Box<dyn Write + Send>,
    ticket: StageTicket,
) -> Box<dyn Write + Send> {
    owned_writer_reporting(inner, ticket, None)
}

/// `owned_writer` whose commit keeps the stage evidence the host reported
/// (`evidence_v1`) for the transfer engine.
pub(super) fn owned_writer_reporting(
    inner: Box<dyn Write + Send>,
    ticket: StageTicket,
    report: Option<WrittenReport>,
) -> Box<dyn Write + Send> {
    Box::new(OwnedStageWriter {
        inner,
        ticket,
        written: 0,
        committed: false,
        failure: None,
        report,
    })
}

struct OwnedStageWriter {
    inner: Box<dyn Write + Send>,
    ticket: StageTicket,
    written: u64,
    committed: bool,
    failure: Option<(io::ErrorKind, String)>,
    report: Option<WrittenReport>,
}

impl OwnedStageWriter {
    fn remember_failure(&mut self, error: io::Error) -> io::Error {
        self.ticket.uncertain();
        self.failure = Some((error.kind(), error.to_string()));
        error
    }

    fn failed(&self) -> io::Result<()> {
        if let Some((kind, message)) = &self.failure {
            return Err(io::Error::new(*kind, message.clone()));
        }
        Ok(())
    }
}

impl Write for OwnedStageWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.failed()?;
        if self.committed {
            return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"));
        }
        match self.inner.write(bytes) {
            Ok(count) => {
                let Some(total) = self.written.checked_add(count as u64) else {
                    return Err(self.remember_failure(io::Error::other("Share-Stufe ist zu groß")));
                };
                self.written = total;
                Ok(count)
            }
            Err(error) => Err(self.remember_failure(error)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.failed()?;
        if self.committed {
            return Ok(());
        }
        if let Err(error) = self
            .inner
            .flush()
            .and_then(|()| self.ticket.committed(self.written))
        {
            return Err(self.remember_failure(error));
        }
        self.committed = true;
        if let Some(report) = &self.report {
            report.record();
        }
        Ok(())
    }
}

impl Drop for OwnedStageWriter {
    fn drop(&mut self) {
        if !self.committed {
            self.ticket.uncertain();
        }
    }
}

/// Where an `evidence_v1` WriteNew commit leaves its `Written` reply.
pub(super) type WrittenSlot = Arc<Mutex<Option<FsWritten>>>;

#[allow(clippy::too_many_arguments)]
pub(super) fn writer(
    node: Arc<ShareIrohNode>,
    send: SendStream,
    recv: RecvStream,
    lease: Option<String>,
    lz4: bool,
    session_key: String,
    generation: usize,
    written: Option<WrittenSlot>,
) -> Box<dyn Write + Send> {
    track_client(&node, &session_key);
    stats::stream_started();
    Box::new(PeerWriter {
        node,
        send: Some(send),
        recv: Some(recv),
        lease,
        lz4: lz4.then(Lz4Stream::new),
        session_key,
        generation,
        state: WriterState::Open,
        written,
    })
}

enum WriterState {
    Open,
    Committed,
    Failed(io::ErrorKind, String),
}

struct PeerWriter {
    node: Arc<ShareIrohNode>,
    send: Option<SendStream>,
    recv: Option<RecvStream>,
    lease: Option<String>,
    /// Packs the chunks when the request asked for LZ4 (`lz4_v1`).
    lz4: Option<Lz4Stream>,
    session_key: String,
    generation: usize,
    state: WriterState,
    /// Receives the `Written` reply of an `evidence_v1` commit.
    written: Option<WrittenSlot>,
}

impl PeerWriter {
    fn finish(&mut self) -> io::Result<()> {
        match &self.state {
            WriterState::Committed => return Ok(()),
            WriterState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriterState::Open => {}
        }
        let Some(mut send) = self.send.take() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        let Some(mut recv) = self.recv.take() else {
            return Err(eio("Peer-Schreibantwort geschlossen"));
        };
        let result = self.node.block_on_route(
            self.generation,
            super::io_deadline::run("peer write finish", async {
                send_ctrl(
                    &mut send,
                    &Ctrl::Fs {
                        req: FsRequest::WriteDone,
                        lease: self.lease.clone(),
                    },
                )
                .await?;
                recv_resp_wire(&mut recv).await
            }),
        );
        match result {
            Ok(response) => match decode_resp(response) {
                Ok(FsResponse::Ok) => self.committed_stream(&mut send, &mut recv),
                Ok(FsResponse::Written { written }) => {
                    self.keep_written(written);
                    self.committed_stream(&mut send, &mut recv)
                }
                Ok(_) => Err(self.fail_stream_parts(
                    &mut send,
                    &mut recv,
                    eio("unerwartete Antwort auf Schreib-Ende"),
                )),
                Err(error) => Err(self.remember_failure(error)),
            },
            Err(error) => Err(self.fail_stream_parts(&mut send, &mut recv, error)),
        }
    }

    /// The host acknowledged the commit: our side of the stream ends.
    fn committed_stream(&mut self, send: &mut SendStream, recv: &mut RecvStream) -> io::Result<()> {
        match send.finish().map_err(eio) {
            Ok(()) => {
                self.state = WriterState::Committed;
                Ok(())
            }
            Err(error) => Err(self.fail_stream_parts(send, recv, error)),
        }
    }

    fn keep_written(&self, written: FsWritten) {
        if let Some(Ok(mut slot)) = self.written.as_ref().map(|slot| slot.lock()) {
            *slot = Some(written);
        }
    }

    fn remember_failure(&mut self, error: io::Error) -> io::Error {
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }

    fn fail_stream_parts(
        &mut self,
        send: &mut SendStream,
        recv: &mut RecvStream,
        error: io::Error,
    ) -> io::Error {
        super::io_deadline::abort(send, recv);
        let _ = self
            .node
            .invalidate_outgoing_session(&self.session_key, self.generation);
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }

    fn fail_open_stream(&mut self, error: io::Error) -> io::Error {
        if let (Some(send), Some(recv)) = (self.send.as_mut(), self.recv.as_mut()) {
            super::io_deadline::abort(send, recv);
        }
        self.send.take();
        self.recv.take();
        let _ = self
            .node
            .invalidate_outgoing_session(&self.session_key, self.generation);
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }
}

impl Write for PeerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &self.state {
            WriterState::Committed => {
                return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"))
            }
            WriterState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriterState::Open => {}
        }
        for chunk in buf.chunks(fs::CHUNK) {
            let (tag, bytes) = frame_for(self.lz4.as_ref(), chunk);
            let started = std::time::Instant::now();
            let result = {
                let Some(send) = self.send.as_mut() else {
                    return Err(eio("Peer-Schreibkanal geschlossen"));
                };
                self.node.block_on_route(
                    self.generation,
                    super::io_deadline::run(
                        "peer write data chunk",
                        send_tagged(send, tag, &bytes),
                    ),
                )
            };
            if stats::enabled() {
                stats::meter().add(stats::Phase::NetWait, started.elapsed());
            }
            if let Err(error) = result {
                return Err(self.fail_open_stream(error));
            }
            if let Some(lz4) = &self.lz4 {
                lz4.record_send(chunk.len(), started.elapsed(), std::time::Instant::now());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for PeerWriter {
    fn drop(&mut self) {
        if matches!(self.state, WriterState::Open) {
            // Stream drop/reset makes the host abandon the write without
            // promotion. Only an explicit flush sends WriteDone; safe cleanup
            // additionally requires stable ownership proof.
            drop(self.send.take());
            drop(self.recv.take());
        }
    }
}
