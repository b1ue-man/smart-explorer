//! Client side of PutBatch: the header, the bytes of every entry back to
//! back, then the commit (WriteDone). A failure before the commit publishes
//! nothing (the host removes its stages); only an interrupted commit is
//! ambiguous, and the host's outcome registry resolves it by the batch
//! nonce (K18e). Nothing is ever retried blindly.
use std::io::{self, Read};
use std::ops::Range;
use std::time::{Duration, Instant};

use iroh::endpoint::RecvStream;

use crate::share::core::{eio, random_hex_token};
use crate::share::framing::{recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use crate::share::fs::CHUNK;
use crate::share::fs_error::into_io;
use crate::share::io_deadline::{self, PEER_OP_TIMEOUT};
use crate::share::node_sessions::OpenedPeerStream;
use crate::share::wire::{
    BatchPart, Ctrl, FsBatchOutcome, FsBatchPut, FsBatchStatus, FsRequest, FsResponse,
    NONCE_HEX_LEN,
};
use crate::vfs::{BatchPut, BatchPutOutcome, VfsResult};

use super::peer_transfer::{copy_error, plan_request};
use super::PeerBackend;

/// Publishing costs a few metadata operations per entry on the host (a
/// round trip each when the export is a remote connection); one second per
/// entry on top of the operation deadline covers slow backends.
const COMMIT_PER_ENTRY: Duration = Duration::from_secs(1);

/// A refusing host sends its reason before it stops the stream, so the
/// reason is normally buffered already; five seconds cover a retransmission.
const REFUSAL_WAIT: Duration = Duration::from_secs(5);

/// A commit publishes one entry per metadata operation (milliseconds on a
/// local export); asking twice a second adds at most half a second to the
/// rare ambiguous case.
const STATUS_POLL: Duration = Duration::from_millis(500);

const SHORT_SOURCE: &str = "Quelle lieferte weniger Bytes als angekündigt";

enum PartOutcome {
    /// The host committed: one outcome per entry.
    Committed(Vec<BatchPutOutcome>),
    /// Nothing was published. `aligned`: every byte of the part was read
    /// from the source, so the next part can follow.
    Refused { error: io::Error, aligned: bool },
    /// The commit may have happened and its outcome could not be learned.
    Unknown(io::Error),
}

enum BodyError {
    /// Reading the caller's bytes failed.
    Source(io::Error),
    /// Sending failed; the host may have refused the batch.
    Transport(io::Error),
}

enum Reply {
    Committed(Vec<FsBatchOutcome>),
    Refused(io::Error),
}

impl PeerBackend {
    pub(super) fn put_batch_v1(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        let limits = self.batch_limits_required()?;
        let lease = self.mount_lease_token()?;
        let wire: Vec<FsBatchPut> = entries
            .iter()
            .map(|entry| FsBatchPut {
                path: entry.path.clone(),
                size: entry.size,
            })
            .collect();
        let envelope = put_header("0".repeat(NONCE_HEX_LEN), Vec::new(), lease.clone());
        let parts = plan_request(&envelope, &wire, |entry| entry.size, limits)?;
        let mut outcomes: Vec<Option<BatchPutOutcome>> =
            std::iter::repeat_with(|| None).take(wire.len()).collect();
        for part in parts {
            let stop = match part {
                BatchPart::Oversized(index) => skip_oversized(&wire, index, data, &mut outcomes),
                BatchPart::Items(range) => {
                    let Some(part_entries) = wire.get(range.clone()) else {
                        continue;
                    };
                    match self.put_part(part_entries, data, &lease) {
                        PartOutcome::Committed(list) => {
                            if let Some(slots) = outcomes.get_mut(range) {
                                for (slot, outcome) in slots.iter_mut().zip(list) {
                                    *slot = Some(outcome);
                                }
                            }
                            None
                        }
                        PartOutcome::Refused { error, aligned } => {
                            fill_range(&mut outcomes, range, &error);
                            // A full host is not asked again at once.
                            let congested = crate::vfs::congestion_of(&error).is_some();
                            (!aligned || congested).then_some(error)
                        }
                        PartOutcome::Unknown(error) => return Err(error),
                    }
                }
            };
            if let Some(error) = stop {
                fill_rest(&mut outcomes, &error);
                break;
            }
        }
        Ok(outcomes
            .into_iter()
            .map(|outcome| {
                outcome.unwrap_or_else(|| {
                    BatchPutOutcome::Failed(eio("Eintrag wurde nicht übertragen"))
                })
            })
            .collect())
    }

    fn put_part(
        &self,
        entries: &[FsBatchPut],
        data: &mut dyn Read,
        lease: &Option<String>,
    ) -> PartOutcome {
        let nonce = match random_hex_token::<8>() {
            Ok(nonce) => nonce,
            Err(error) => {
                return PartOutcome::Refused {
                    error: eio(error),
                    aligned: false,
                }
            }
        };
        let mut opened = match self.open_batch_stream() {
            Ok(opened) => opened,
            Err(error) => {
                return PartOutcome::Refused {
                    error,
                    aligned: false,
                }
            }
        };
        let header = put_header(nonce.clone(), entries.to_vec(), lease.clone());
        let total = entries
            .iter()
            .fold(0u64, |total, entry| total.saturating_add(entry.size));
        if let Err(failure) = self.send_body(&mut opened, &header, data, total) {
            return self.refused_part(opened, failure);
        }
        // From here on the host may commit. The replies still decide: a
        // refusal before `Ready` means nothing happened; only a missing
        // outcome after the commit is ambiguous.
        let commit = Ctrl::Fs {
            req: FsRequest::WriteDone,
            lease: lease.clone(),
        };
        let sent = self.node.block_on(io_deadline::run(
            "peer batch commit",
            send_ctrl(&mut opened.send, &commit),
        ));
        let entry_count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        let budget = match &sent {
            Ok(()) => PEER_OP_TIMEOUT.saturating_add(COMMIT_PER_ENTRY.saturating_mul(entry_count)),
            // A host that stopped the stream has sent its reason already.
            Err(_) => REFUSAL_WAIT,
        };
        let reply = self.node.block_on(io_deadline::run_for(
            "peer batch reply",
            budget,
            read_reply(&mut opened.recv),
        ));
        match reply {
            Ok(Reply::Committed(outcomes)) => {
                // Finishing our side tells the host the outcomes arrived.
                let _ = opened.send.finish();
                committed(outcomes, entries.len())
            }
            Ok(Reply::Refused(error)) => PartOutcome::Refused {
                error,
                aligned: true,
            },
            Err(error) => {
                let cause = sent.err().unwrap_or(error);
                self.resolve_unknown(opened, &nonce, entries.len(), cause)
            }
        }
    }

    /// Header, then the entries' bytes back to back in chunk-sized frames.
    fn send_body(
        &self,
        opened: &mut OpenedPeerStream,
        header: &Ctrl,
        data: &mut dyn Read,
        total: u64,
    ) -> Result<(), BodyError> {
        self.node
            .block_on(io_deadline::run(
                "peer batch header",
                send_ctrl(&mut opened.send, header),
            ))
            .map_err(BodyError::Transport)?;
        let mut buffer = vec![0u8; CHUNK];
        let mut remaining = total;
        while remaining > 0 {
            let wanted = usize::try_from(remaining).map_or(CHUNK, |remaining| remaining.min(CHUNK));
            let Some(chunk) = buffer.get_mut(..wanted) else {
                return Err(BodyError::Source(eio("Paketpuffer ist zu klein")));
            };
            let filled = fill(data, chunk).map_err(BodyError::Source)?;
            if filled < wanted {
                return Err(BodyError::Source(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    SHORT_SOURCE,
                )));
            }
            self.node
                .block_on(io_deadline::run(
                    "peer batch data",
                    send_tagged(&mut opened.send, TAG_DATA, chunk),
                ))
                .map_err(BodyError::Transport)?;
            remaining -= wanted as u64;
        }
        Ok(())
    }

    /// Before the commit nothing is published: the stream is reset and the
    /// host removes the stages. A refusing host's reason is reported.
    fn refused_part(&self, mut opened: OpenedPeerStream, failure: BodyError) -> PartOutcome {
        let error = match failure {
            BodyError::Source(error) => error,
            BodyError::Transport(error) => {
                let reason = self.node.block_on(io_deadline::run_for(
                    "peer batch refusal",
                    REFUSAL_WAIT,
                    recv_resp_wire(&mut opened.recv),
                ));
                match reason {
                    Ok(FsResponse::Err { kind, msg }) => into_io(kind, msg),
                    _ => {
                        let _ = self
                            .node
                            .invalidate_outgoing_session(&opened.session_key, opened.generation);
                        error
                    }
                }
            }
        };
        io_deadline::abort(&mut opened.send, &mut opened.recv);
        PartOutcome::Refused {
            error,
            aligned: false,
        }
    }

    /// The commit may have happened: ask the host by the nonce until it
    /// answers or one operation deadline has passed.
    fn resolve_unknown(
        &self,
        mut opened: OpenedPeerStream,
        nonce: &str,
        count: usize,
        error: io::Error,
    ) -> PartOutcome {
        io_deadline::abort(&mut opened.send, &mut opened.recv);
        let _ = self
            .node
            .invalidate_outgoing_session(&opened.session_key, opened.generation);
        let deadline = Instant::now() + PEER_OP_TIMEOUT;
        loop {
            let status = self.request(FsRequest::PutBatchStatus {
                nonce: nonce.to_string(),
            });
            match status {
                Ok(FsResponse::Batch {
                    status: FsBatchStatus::Done { outcomes },
                }) => return committed(outcomes, count),
                Ok(FsResponse::Batch {
                    status: FsBatchStatus::Aborted,
                }) => {
                    return PartOutcome::Refused {
                        error: eio(format!(
                            "Paket wurde vor dem Veröffentlichen abgebrochen: {error}"
                        )),
                        aligned: true,
                    }
                }
                Ok(FsResponse::Batch {
                    status: FsBatchStatus::Pending,
                }) => {}
                Ok(other) => {
                    let cause = format!("{error}; unerwartete Statusantwort {other:?}");
                    return PartOutcome::Unknown(unknown(nonce, &cause));
                }
                // Unreachable for now (reconnecting, busy): ask again.
                Err(status) if retry_status(&status) => {}
                // The host answered without an outcome (unknown nonce,
                // revoked access): it stays open.
                Err(status) => {
                    let cause = format!("{error}; {status}");
                    return PartOutcome::Unknown(unknown(nonce, &cause));
                }
            }
            if Instant::now() >= deadline {
                return PartOutcome::Unknown(unknown(nonce, &error.to_string()));
            }
            std::thread::sleep(STATUS_POLL);
        }
    }
}

fn put_header(nonce: String, entries: Vec<FsBatchPut>, lease: Option<String>) -> Ctrl {
    Ctrl::Fs {
        req: FsRequest::PutBatch { nonce, entries },
        lease,
    }
}

/// `Ready` (or a refusal), then the commit's outcomes (or a refusal).
async fn read_reply(recv: &mut RecvStream) -> io::Result<Reply> {
    match recv_resp_wire(recv).await? {
        FsResponse::Ready => {}
        FsResponse::Err { kind, msg } => return Ok(Reply::Refused(into_io(kind, msg))),
        _ => return Err(eio("unerwartete Antwort auf das Paket")),
    }
    match recv_resp_wire(recv).await? {
        FsResponse::Batch {
            status: FsBatchStatus::Done { outcomes },
        } => Ok(Reply::Committed(outcomes)),
        FsResponse::Batch { status } => Err(eio(format!(
            "unerwarteter Paketstatus: {}",
            status.summary()
        ))),
        FsResponse::Err { kind, msg } => Ok(Reply::Refused(into_io(kind, msg))),
        _ => Err(eio("unerwartete Antwort auf den Paketabschluss")),
    }
}

fn committed(outcomes: Vec<FsBatchOutcome>, count: usize) -> PartOutcome {
    if outcomes.len() != count {
        return PartOutcome::Unknown(eio(format!(
            "Host meldet {} statt {count} Paketergebnisse",
            outcomes.len()
        )));
    }
    PartOutcome::Committed(
        outcomes
            .into_iter()
            .map(|outcome| match outcome {
                FsBatchOutcome::Published { path } => BatchPutOutcome::Published(path),
                FsBatchOutcome::Failed { kind, msg } => BatchPutOutcome::Failed(into_io(kind, msg)),
            })
            .collect(),
    )
}

/// Failures of reaching the host, not answers of the host.
fn retry_status(error: &io::Error) -> bool {
    crate::vfs::congestion_of(error).is_some()
        || matches!(
            error.kind(),
            io::ErrorKind::TimedOut
                | io::ErrorKind::NotConnected
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::Interrupted
        )
}

fn unknown(nonce: &str, cause: &str) -> io::Error {
    io::Error::other(format!(
        "Ergebnis des Pakets {nonce} ist unbekannt; es wird nicht wiederholt: {cause}"
    ))
}

/// Reads until `buffer` is full or the source ends; returns the bytes read.
fn fill(data: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while let Some(rest) = buffer.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match data.read(rest) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Skips the bytes of an entry too large for any batch; the caller sends it
/// on its own. `Some` stops the call when the source cannot be skipped.
fn skip_oversized(
    wire: &[FsBatchPut],
    index: usize,
    data: &mut dyn Read,
    outcomes: &mut [Option<BatchPutOutcome>],
) -> Option<io::Error> {
    let size = wire.get(index).map_or(0, |entry| entry.size);
    match io::copy(&mut Read::take(&mut *data, size), &mut io::sink()) {
        Ok(skipped) if skipped == size => {
            if let Some(slot) = outcomes.get_mut(index) {
                *slot = Some(BatchPutOutcome::Failed(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Datei passt in kein Paket; sie wird einzeln übertragen",
                )));
            }
            None
        }
        Ok(_) => Some(io::Error::new(io::ErrorKind::UnexpectedEof, SHORT_SOURCE)),
        Err(error) => Some(error),
    }
}

fn fill_range(outcomes: &mut [Option<BatchPutOutcome>], range: Range<usize>, error: &io::Error) {
    if let Some(slots) = outcomes.get_mut(range) {
        for slot in slots {
            *slot = Some(BatchPutOutcome::Failed(copy_error(error)));
        }
    }
}

fn fill_rest(outcomes: &mut [Option<BatchPutOutcome>], error: &io::Error) {
    for slot in outcomes.iter_mut().filter(|slot| slot.is_none()) {
        *slot = Some(BatchPutOutcome::Failed(copy_error(error)));
    }
}
