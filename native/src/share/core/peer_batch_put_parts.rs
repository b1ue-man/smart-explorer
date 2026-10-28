//! Pieces of the client PutBatch: its header, the host's replies, the
//! outcomes per entry and the entries that never reached the host.
use std::io::{self, Read};
use std::ops::Range;

use iroh::endpoint::RecvStream;

use crate::share::core::eio;
use crate::share::framing::recv_resp_wire;
use crate::share::fs_error::into_io;
use crate::share::wire::{Ctrl, FsBatchOutcome, FsBatchPut, FsBatchStatus, FsRequest, FsResponse};
use crate::vfs::{EvidencePutOutcome, VfsResult};

use super::super::peer_evidence::EvidenceBinding;
use super::super::peer_transfer::copy_error;
use super::super::PeerBackend;

pub(super) const SHORT_SOURCE: &str = "Quelle lieferte weniger Bytes als angekündigt";

pub(super) enum PartOutcome {
    /// The host committed: one outcome per entry.
    Committed(Vec<EvidencePutOutcome>),
    /// Nothing was published. `aligned`: every byte of the part was read
    /// from the source, so the next part can follow.
    Refused { error: io::Error, aligned: bool },
    /// The commit may have happened and its outcome could not be learned.
    Unknown(io::Error),
}

pub(super) enum Reply {
    Committed(Vec<FsBatchOutcome>),
    Refused(io::Error),
}

pub(super) fn put_header(
    nonce: String,
    entries: Vec<FsBatchPut>,
    lease: Option<String>,
    evidence: bool,
) -> Ctrl {
    Ctrl::Fs {
        req: FsRequest::PutBatch {
            nonce,
            entries,
            evidence,
        },
        lease,
    }
}

/// `Ready` (or a refusal), then the commit's outcomes (or a refusal).
pub(super) async fn read_reply(recv: &mut RecvStream) -> io::Result<Reply> {
    match recv_resp_wire(recv).await? {
        FsResponse::Ready => {}
        FsResponse::Err { kind, msg } => return Ok(Reply::Refused(into_io(kind, msg))),
        _ => return Err(eio("unerwartete Antwort auf das Paket")),
    }
    read_outcome(recv).await
}

/// The commit's outcomes (or a refusal); an `evidence_v1` batch read its
/// `BatchReady` before the commit already.
pub(super) async fn read_outcome(recv: &mut RecvStream) -> io::Result<Reply> {
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

/// One outcome per entry; the evidence of a published entry is kept only
/// while the connection's binding is unchanged (`binding`).
pub(super) fn committed(
    backend: &PeerBackend,
    outcomes: Vec<FsBatchOutcome>,
    count: usize,
    binding: Option<&EvidenceBinding>,
) -> PartOutcome {
    if outcomes.len() != count {
        return PartOutcome::Unknown(eio(format!(
            "Host meldet {} statt {count} Paketergebnisse",
            outcomes.len()
        )));
    }
    let binding = binding.filter(|binding| binding.still_current(backend));
    PartOutcome::Committed(
        outcomes
            .into_iter()
            .map(|outcome| match outcome {
                FsBatchOutcome::Published { path, evidence } => {
                    let evidence = match (binding, evidence) {
                        (Some(binding), Some(evidence)) => binding.file(&path, evidence),
                        _ => None,
                    };
                    EvidencePutOutcome::Published { path, evidence }
                }
                FsBatchOutcome::Failed { kind, msg } => {
                    EvidencePutOutcome::Failed(into_io(kind, msg))
                }
            })
            .collect(),
    )
}

/// Failures of reaching the host, not answers of the host.
pub(super) fn retry_status(error: &io::Error) -> bool {
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

pub(super) fn unknown(nonce: &str, cause: &str) -> io::Error {
    io::Error::other(format!(
        "Ergebnis des Pakets {nonce} ist unbekannt; es wird nicht wiederholt: {cause}"
    ))
}

/// A failure before the commit was sent: the host cannot have published
/// anything of the part. Congestion stays congestion.
pub(super) fn before_commit(error: io::Error) -> io::Error {
    if crate::vfs::congestion_of(&error).is_some() || crate::vfs::unpublished_of(&error) {
        return error;
    }
    crate::vfs::unpublished_error(error.kind(), error.to_string())
}

/// Reads until `buffer` is full or the source ends; returns the bytes read.
pub(super) fn fill(data: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
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
pub(super) fn skip_oversized(
    wire: &[FsBatchPut],
    index: usize,
    data: &mut dyn Read,
    outcomes: &mut [Option<EvidencePutOutcome>],
) -> Option<io::Error> {
    let size = wire.get(index).map_or(0, |entry| entry.size);
    match io::copy(&mut Read::take(&mut *data, size), &mut io::sink()) {
        Ok(skipped) if skipped == size => {
            if let Some(slot) = outcomes.get_mut(index) {
                *slot = Some(EvidencePutOutcome::Failed(io::Error::new(
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

pub(super) fn fill_range(
    outcomes: &mut [Option<EvidencePutOutcome>],
    range: Range<usize>,
    error: &io::Error,
) {
    if let Some(slots) = outcomes.get_mut(range) {
        for slot in slots {
            *slot = Some(EvidencePutOutcome::Failed(copy_error(error)));
        }
    }
}

pub(super) fn fill_rest(outcomes: &mut [Option<EvidencePutOutcome>], error: &io::Error) {
    for slot in outcomes.iter_mut().filter(|slot| slot.is_none()) {
        *slot = Some(EvidencePutOutcome::Failed(copy_error(error)));
    }
}

/// The outcomes as `put_batch` reports them (without evidence).
pub(super) fn plain(
    outcomes: Vec<EvidencePutOutcome>,
) -> VfsResult<Vec<crate::vfs::BatchPutOutcome>> {
    Ok(outcomes
        .into_iter()
        .map(|outcome| match outcome {
            EvidencePutOutcome::Published { path, .. } => {
                crate::vfs::BatchPutOutcome::Published(path)
            }
            EvidencePutOutcome::Failed(error) => crate::vfs::BatchPutOutcome::Failed(error),
        })
        .collect())
}
