//! One new file as a packet of one (Z-P4): for a peer that offers it
//! (`vfs::single_put`), a single upload goes as a PutBatch with one entry
//! instead of WriteNew, WriteDone, the receipt's stage queries, the
//! publication and the queries after it. With transfer evidence that is one
//! round trip for the whole file.
//!
//! The invariants of the single path hold:
//! * the intent is durable before anything can be published: without
//!   evidence it is written before the packet starts; with evidence the
//!   peer names whether the destination existed (`BatchReady`) and the
//!   intent is written then, before the commit goes out;
//! * the peer publishes without replacing, numbered when the name is taken,
//!   and answers with the result (and its evidence);
//! * a lost answer is resolved by the peer's outcome registry (the packet
//!   nonce, `PutBatchStatus`); if it stays unknown the file is reported as
//!   possibly published and is never sent again;
//! * a file that certainly was not published has its intent withdrawn and
//!   takes the worker's ordinary retry, then the classic path.
use super::super::super::engine_names::{base_name, parent_rel};
use super::super::batch::Attempt;
use super::super::ops::{relabeled, At, Meter, OpError, OpResult, Outcome};
use super::super::queue::FileWork;
use super::super::source::LocalSource;
use super::super::Engine;
use super::{host_evidence, prepare_part, without_evidence, PacketData};
use crate::vfs::{Backend, BatchPut, EvidencePutOutcome};
use std::io;

/// `Some`: `file` went as a packet of one and this is its result; `None`:
/// the peer does not take it so (or this file must go the classic way) and
/// nothing was sent.
pub(in super::super) fn upload(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    source: &mut LocalSource,
    meter: &Meter<'_>,
) -> Option<OpResult> {
    let limits = engine.batch?;
    let size = source.length();
    if size > limits.max_bytes || !eligible(engine, file, source) {
        return None;
    }
    if !crate::vfs::single_put(target) {
        return None;
    }
    let files = std::slice::from_ref(file);
    let entries = [BatchPut {
        path: engine.folders.path_of(&file.rel),
        size,
        // The source's time, where the peer sets it (`publish_time.rs`).
        mtime_ms: (file.mtime_ms > 0).then_some(file.mtime_ms),
    }];
    let evidenced = crate::vfs::transfer_evidence(target, engine.folders.root());
    if !evidenced {
        if let Err(error) = prepare_part(engine, files, 0, &[None]) {
            return Some(preparation_failed(engine, file, error));
        }
    }
    let mut prepare_failure: Option<io::Error> = None;
    let (result, delivery) = {
        let mut data = PacketData::new(vec![source], meter, engine.stop_flag());
        let result = if evidenced {
            let mut prepare = |offset: usize, existed: &[Option<bool>]| {
                prepare_part(engine, files, offset, existed).inspect_err(|error| {
                    prepare_failure = Some(io::Error::new(error.kind(), error.to_string()));
                })
            };
            crate::vfs::put_batch_evidence(target, &entries, &mut data, &mut prepare)
        } else {
            target.put_batch(&entries, &mut data).map(without_evidence)
        };
        (result, data.delivery())
    };
    let untouched = delivery.served.iter().all(|served| *served == 0);
    // The intent could not be written: the commit never went out.
    if let Some(error) = prepare_failure {
        return Some(preparation_failed(engine, file, error));
    }
    Some(match result {
        Ok(outcomes) => match outcomes.into_iter().next() {
            Some(EvidencePutOutcome::Published { path, evidence }) => {
                published(engine, file, &path, evidence)
            }
            Some(EvidencePutOutcome::Failed(error)) => {
                let (failure, mut attempt) = delivery.judge(0, OpError::target(error));
                // The peer says it published nothing for this file. A
                // refusal as too busy (Share "Busy", `admit_now`) always
                // comes before the commit: nothing was published, and the
                // worker waits and sends again, as for packets (K13).
                if attempt == Attempt::MaybePublished
                    && (crate::vfs::unpublished_of(&failure.error)
                        || crate::vfs::congestion_of(&failure.error).is_some())
                {
                    attempt = Attempt::NotPublished;
                }
                failed(engine, file, failure, attempt)
            }
            None => {
                let missing = io::Error::other("Upload lieferte kein Ergebnis für diese Datei");
                let (failure, attempt) = delivery.judge(0, OpError::at(At::Unknown, missing));
                failed(engine, file, failure, attempt)
            }
        },
        // Refused before anything was read: the classic path takes it.
        Err(error) if error.kind() == io::ErrorKind::Unsupported && untouched => {
            return match super::super::file_recovery::unpublished(engine, file) {
                Ok(()) => None,
                Err(failure) => Some(Err(OpError::at(At::Unknown, failure))),
            };
        }
        Err(error) => {
            let lost = OpError::at(At::Unknown, relabeled(&error, "Upload"));
            let (failure, attempt) = delivery.judge(0, lost);
            failed(engine, file, failure, attempt)
        }
    })
}

/// Files the classic path decides: a retried file or one sent alone after
/// a packet, "transfer missing files", a resumed file with a receipt of its
/// own (`file_recovery::resume`) and optical sources (read once, staged).
fn eligible(engine: &Engine<'_>, file: &FileWork, source: &LocalSource) -> bool {
    if file.retried || file.alone || engine.resume || source.is_optical() {
        return false;
    }
    engine.view.recovery != crate::transfer::RecoveryMode::Resume
        || !super::super::file_recovery::has_receipt(engine, file)
}

fn published(
    engine: &Engine<'_>,
    file: &FileWork,
    path: &str,
    evidence: Option<crate::vfs::FileEvidence>,
) -> OpResult {
    let acknowledged = match evidence {
        Some(evidence) => super::super::file_recovery::after_known(
            engine,
            file,
            path,
            Some(&host_evidence(evidence)),
        ),
        None => super::super::file_recovery::after(engine, file, path),
    };
    acknowledged.map_err(|error| OpError::at(At::Unknown, error))?;
    if parent_rel(&file.rel).is_none() {
        engine.folders.record_alias(&file.rel, base_name(path));
    }
    Ok(Outcome::Done)
}

/// A failed packet of one: a file that may have been published keeps its
/// intent and is reported (never sent again); any other withdraws it and
/// returns a failure before publication, which the worker may retry.
fn failed(engine: &Engine<'_>, file: &FileWork, failure: OpError, attempt: Attempt) -> OpResult {
    match attempt {
        Attempt::MaybePublished => Err(OpError::at(At::Unknown, failure.error)),
        Attempt::NotPublished | Attempt::Untried => {
            if let Err(error) = super::super::file_recovery::unpublished(engine, file) {
                return Err(OpError::at(
                    At::Unknown,
                    io::Error::other(format!(
                        "Unveröffentlichten Dateibeleg zurücknehmen: {error}; {}",
                        failure.error
                    )),
                ));
            }
            match failure.at {
                At::Source | At::Target => Err(failure),
                // Not all bytes went out: nothing was published.
                _ => Err(OpError::at(At::Target, failure.error)),
            }
        }
    }
}

/// As `publish::publish` when the intent cannot be written: nothing was
/// published, the failure is the target's.
fn preparation_failed(engine: &Engine<'_>, file: &FileWork, error: io::Error) -> OpResult {
    if let Err(failure) = super::super::file_recovery::preparation_failed(engine, file) {
        return Err(OpError::at(
            At::Unknown,
            io::Error::other(format!(
                "Publikationsvorbereitung fehlgeschlagen: {error}; Nichtpublikation unbestätigt: {failure}"
            )),
        ));
    }
    Err(OpError::target(error))
}
