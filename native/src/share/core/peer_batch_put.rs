//! Client side of PutBatch: the header, the bytes of every entry back to
//! back, then the commit (WriteDone). A failure before the commit publishes
//! nothing (the host removes its stages); only an interrupted commit is
//! ambiguous, and the host's outcome registry resolves it by the batch
//! nonce (K18e). Nothing is ever retried blindly. With `evidence_v1` the
//! host names the existing destinations (`BatchReady`) before the commit,
//! the caller's `prepare` records them first, and every published outcome
//! carries the published file's evidence.
use std::io::{self, Read};
use std::time::{Duration, Instant};

use crate::share::core::{eio, random_hex_token};
use crate::share::framing::{recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use crate::share::fs::CHUNK;
use crate::share::fs_error::into_io;
use crate::share::io_deadline::{self, PEER_OP_TIMEOUT};
use crate::share::node_sessions::OpenedPeerStream;
use crate::share::wire::{
    BatchPart, Ctrl, FsBatchPut, FsBatchStatus, FsRequest, FsResponse, NONCE_HEX_LEN,
};
use crate::vfs::{BatchPut, BatchPutOutcome, EvidencePutOutcome, PreparePut, VfsResult};

use super::peer_evidence::EvidenceBinding;
use super::peer_transfer::plan_request;
use super::PeerBackend;

#[path = "peer_batch_put_parts.rs"]
mod parts;
use parts::{
    before_commit, committed, fill, fill_range, fill_rest, plain, put_header, read_outcome,
    read_reply, retry_status, skip_oversized, unknown, PartOutcome, Reply, SHORT_SOURCE,
};

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

enum BodyError {
    /// Reading the caller's bytes failed.
    Source(io::Error),
    /// Sending failed; the host may have refused the batch.
    Transport(io::Error),
}

impl PeerBackend {
    pub(super) fn put_batch_v1(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        plain(self.put_batch_parts(entries, data, None)?)
    }

    /// `evidence_v1` PutBatch (`vfs::put_batch_evidence`); `Unsupported`
    /// before anything is read where the host does not offer it.
    pub(in crate::share) fn put_batch_evidenced(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
        prepare: &mut PreparePut<'_>,
    ) -> VfsResult<Vec<EvidencePutOutcome>> {
        if !self.evidence_offered() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Die Gegenstelle meldet keine Übertragungsbelege",
            ));
        }
        self.put_batch_parts(entries, data, Some(prepare))
    }

    fn put_batch_parts(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
        mut prepare: Option<&mut PreparePut<'_>>,
    ) -> VfsResult<Vec<EvidencePutOutcome>> {
        let limits = self.batch_limits_required()?;
        let lease = self.mount_lease_token()?;
        let evidence = prepare.is_some();
        let wire: Vec<FsBatchPut> = entries
            .iter()
            .map(|entry| FsBatchPut {
                path: entry.path.clone(),
                size: entry.size,
            })
            .collect();
        let envelope = put_header(
            "0".repeat(NONCE_HEX_LEN),
            Vec::new(),
            lease.clone(),
            evidence,
        );
        let parts = plan_request(&envelope, &wire, |entry| entry.size, limits)?;
        let mut outcomes: Vec<Option<EvidencePutOutcome>> =
            std::iter::repeat_with(|| None).take(wire.len()).collect();
        for part in parts {
            let stop = match part {
                BatchPart::Oversized(index) => skip_oversized(&wire, index, data, &mut outcomes),
                BatchPart::Items(range) => {
                    let Some(part_entries) = wire.get(range.clone()) else {
                        continue;
                    };
                    let part_prepare = prepare.as_deref_mut();
                    match self.put_part(part_entries, data, &lease, range.start, part_prepare) {
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
                    EvidencePutOutcome::Failed(eio("Eintrag wurde nicht übertragen"))
                })
            })
            .collect())
    }

    fn put_part(
        &self,
        entries: &[FsBatchPut],
        data: &mut dyn Read,
        lease: &Option<String>,
        offset: usize,
        prepare: Option<&mut PreparePut<'_>>,
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
        // The connection the evidence identities are bound to; a batch
        // whose binding changed keeps its outcomes without evidence.
        let binding = match prepare.is_some().then(|| EvidenceBinding::capture(self)) {
            Some(Err(error)) => {
                return PartOutcome::Refused {
                    error: before_commit(error),
                    aligned: false,
                }
            }
            Some(Ok(binding)) => Some(binding),
            None => None,
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
        let header = put_header(
            nonce.clone(),
            entries.to_vec(),
            lease.clone(),
            prepare.is_some(),
        );
        let total = entries
            .iter()
            .fold(0u64, |total, entry| total.saturating_add(entry.size));
        if let Err(failure) = self.send_body(&mut opened, &header, data, total) {
            return self.refused_part(opened, failure);
        }
        let evidence = prepare.is_some();
        if let Some(prepare) = prepare {
            if let Some(refused) = self.await_ready(&mut opened, offset, entries.len(), prepare) {
                return refused;
            }
        }
        // From here on the host may commit. The replies still decide: a
        // refusal before `Ready` means nothing happened; only a missing
        // outcome after the commit is ambiguous.
        let commit = Ctrl::Fs {
            req: FsRequest::WriteDone,
            lease: lease.clone(),
        };
        let sent = self.node.block_on_route(
            opened.generation,
            io_deadline::run("peer batch commit", send_ctrl(&mut opened.send, &commit)),
        );
        let entry_count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
        let budget = match &sent {
            Ok(()) => PEER_OP_TIMEOUT.saturating_add(COMMIT_PER_ENTRY.saturating_mul(entry_count)),
            // A host that stopped the stream has sent its reason already.
            Err(_) => REFUSAL_WAIT,
        };
        let reply = if evidence {
            self.node.block_on_route(
                opened.generation,
                io_deadline::run_for("peer batch reply", budget, read_outcome(&mut opened.recv)),
            )
        } else {
            self.node.block_on_route(
                opened.generation,
                io_deadline::run_for("peer batch reply", budget, read_reply(&mut opened.recv)),
            )
        };
        match reply {
            Ok(Reply::Committed(outcomes)) => {
                // Finishing our side tells the host the outcomes arrived.
                let _ = opened.send.finish();
                committed(self, outcomes, entries.len(), binding.as_ref())
            }
            Ok(Reply::Refused(error)) => PartOutcome::Refused {
                error,
                aligned: true,
            },
            Err(error) => {
                let cause = sent.err().unwrap_or(error);
                self.resolve_unknown(opened, &nonce, entries.len(), cause, binding.as_ref())
            }
        }
    }

    /// `evidence_v1`: the host's `BatchReady` (sent as soon as it accepted
    /// the part, so it is normally buffered by now), then the caller's
    /// `prepare`. `Some` ends the part before its commit: nothing of it is
    /// published, and the host removes its stages when the stream resets.
    fn await_ready(
        &self,
        opened: &mut OpenedPeerStream,
        offset: usize,
        count: usize,
        prepare: &mut PreparePut<'_>,
    ) -> Option<PartOutcome> {
        let reply = self.node.block_on_route(
            opened.generation,
            io_deadline::run("peer batch ready", recv_resp_wire(&mut opened.recv)),
        );
        let failure = match reply {
            Ok(FsResponse::BatchReady { existed }) if existed.len() == count => {
                match prepare(offset, &existed) {
                    Ok(()) => return None,
                    // Our own refusal: the connection stays.
                    Err(error) => {
                        io_deadline::abort(&mut opened.send, &mut opened.recv);
                        return Some(PartOutcome::Refused {
                            error: before_commit(error),
                            aligned: false,
                        });
                    }
                }
            }
            // The host refused before it accepted anything (`Busy`).
            Ok(FsResponse::Err { kind, msg }) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                return Some(PartOutcome::Refused {
                    error: before_commit(into_io(kind, msg)),
                    aligned: true,
                });
            }
            Ok(other) => eio(format!("unerwartete Paketannahme: {other:?}")),
            Err(error) => error,
        };
        io_deadline::abort(&mut opened.send, &mut opened.recv);
        let _ = self
            .node
            .invalidate_outgoing_session(&opened.session_key, opened.generation);
        Some(PartOutcome::Refused {
            error: before_commit(failure),
            aligned: false,
        })
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
            .block_on_route(
                opened.generation,
                io_deadline::run("peer batch header", send_ctrl(&mut opened.send, header)),
            )
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
                .block_on_route(
                    opened.generation,
                    io_deadline::run(
                        "peer batch data",
                        send_tagged(&mut opened.send, TAG_DATA, chunk),
                    ),
                )
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
                let reason = self.node.block_on_route(
                    opened.generation,
                    io_deadline::run_for(
                        "peer batch refusal",
                        REFUSAL_WAIT,
                        recv_resp_wire(&mut opened.recv),
                    ),
                );
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
        binding: Option<&EvidenceBinding>,
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
                }) => return committed(self, outcomes, count, binding),
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
