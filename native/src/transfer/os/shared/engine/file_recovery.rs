//! Per-file publication receipts. The live path records the source, the
//! destination and (for a local stage) the stage's identity before
//! publication, without reading any file back. A resumed or retried job
//! decides ownership in doubt by content digest (`doubt`); equal size alone
//! never establishes it. Local copies and moves keep their content proofs.
//!
//! The receipts live in the job's crash log (`journal`, `crash_log`): an intent is
//! durable before its publication, an acknowledgement follows without its
//! own sync. The `*_known` variants take evidence a host reply already
//! carried (`HostEvidence`) and then send no stat or identity request of
//! their own.
use super::file_evidence::{verify, HostEvidence, Proof};
use super::job_store::{endpoint, snapshot, snapshot_checked, with_backend, JobRun};
use super::queue::FileWork;
use super::record_io::{read_record, write_record, RecordContext, RecordKind};
use super::view::{JobView, Side};
use super::Engine;
use crate::transfer::job_record::{JobRecord, SourceGuard, StoredEndpoint};
#[cfg(test)]
#[path = "batches_publication_compatibility_task_tests.rs"]
mod compatibility_tests;
#[cfg(test)]
#[path = "file_recovery_doubt_tests.rs"]
mod doubt_tests;
#[cfg(test)]
#[path = "file_recovery_log_tests.rs"]
mod log_tests;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "file_recovery_log.rs"]
mod crash_log;
#[path = "file_recovery_doubt.rs"]
mod doubt;
#[path = "file_recovery_journal.rs"]
mod journal;
#[path = "file_recovery_legacy.rs"]
mod legacy;
#[path = "missing_retry.rs"]
pub(super) mod missing_retry;
#[path = "file_recovery_observer.rs"]
mod observer;
#[path = "file_recovery_paths.rs"]
mod paths;
#[path = "file_recovery_receipts.rs"]
mod receipts;
#[path = "file_recovery_state.rs"]
mod state;
#[path = "file_recovery_verify.rs"]
mod verification;

#[cfg(test)]
pub(super) use journal::log_path;
pub(super) use journal::Journal;
#[cfg(test)]
pub(super) use legacy::{convert_to_old_format, old_receipt_file};
pub(super) use observer::{stage_proof, LocalObserver};
use paths::destination_parts;
use receipts::{PreviousTarget, Receipt};
pub(super) use verification::verify_saved;
use verification::{previous_presence, verify_source};

fn failure(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn active(engine: &Engine<'_>) -> io::Result<()> {
    if engine.stopped() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            super::super::cancel::CANCELED_ERROR,
        ))
    } else {
        Ok(())
    }
}

pub(super) type FileRoots =
    std::collections::BTreeMap<String, (SourceGuard, crate::transfer::ResolvedRoot)>;

pub(super) use missing_retry::resume;

#[path = "file_recovery_packet.rs"]
mod packet;
pub(super) use packet::{prepare_batch_known, PacketIntent};

/// The source as recorded: from the host's reply when it carried it,
/// otherwise looked at (one stat and identity request for a remote source).
fn source_guard(
    engine: &Engine<'_>,
    file: &FileWork,
    known: Option<&HostEvidence>,
) -> io::Result<SourceGuard> {
    let Some(evidence) = known else {
        // A failed stat keeps its own error: an overload of the source's
        // peer (many files at once through the proxy) waits and repeats
        // instead of ending the file as unconfirmed.
        return snapshot_checked(engine.view.source, &file.source);
    };
    if evidence.path != file.source {
        return Err(failure("Hostbeleg der Quelle passt nicht zur Datei"));
    }
    Ok(SourceGuard {
        path: file.source.clone(),
        directory: false,
        link: false,
        id: evidence.id.clone(),
        object: evidence.object.clone(),
        size: evidence.size,
        mtime_ms: evidence.mtime_ms,
        verified: true,
    })
}

/// What was at `destination` before: as the host reported it, otherwise
/// looked at (one stat request on a remote target).
fn previous_known(
    side: Side<'_>,
    destination: &str,
    existed_before: Option<bool>,
) -> io::Result<PreviousTarget> {
    match existed_before {
        Some(false) => Ok(PreviousTarget::Absent),
        Some(true) => Ok(PreviousTarget::Unconfirmed),
        None => previous_presence(side, destination),
    }
}

pub(super) fn before(
    engine: &Engine<'_>,
    file: &FileWork,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    before_known(engine, file, staged, destination, None, None, None)
}

/// `before` with evidence a host reply already carried: the remote source
/// (`source`), the stage (`stage`) and whether `destination` existed before.
/// Each `None` is looked up as `before` does.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn before_known(
    engine: &Engine<'_>,
    file: &FileWork,
    staged: &str,
    destination: &str,
    source: Option<&HostEvidence>,
    stage: Option<&HostEvidence>,
    existed_before: Option<bool>,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    active(engine)?;
    let source = source_guard(engine, file, source)?;
    if !source.verified || source.directory || source.link {
        return Err(failure("Quelle des Dateibelegs ist unbestätigt"));
    }
    // No content proof by reading the file back (user 2026-10-27: far too
    // slow, it read every file back twice): the transport delivers every byte
    // integrity-protected and acknowledges each write; size and kind of the
    // stage are checked. A resumed job decides ownership by metadata, in
    // doubt by content digest (`doubt`); the stage is recorded by metadata
    // so a crash removes exactly it.
    let stage = match stage {
        Some(evidence) => known_stage(engine, file, source.size, staged, evidence)?,
        None => {
            let meta = super::file_evidence::validate_copy_target(
                engine.view.source,
                &file.source,
                source.size,
                engine.view.target,
                staged,
            )?;
            super::file_evidence::stage_record(engine.view.target, staged, meta)?
        }
    };
    record.start_file(
        file,
        source,
        stage,
        destination.to_string(),
        None,
        previous_known(engine.view.target, destination, existed_before)?,
    )?;
    active(engine)
}

/// A stage as the host described it, checked like `validate_copy_target`
/// and recorded like `stage_record`.
fn known_stage(
    engine: &Engine<'_>,
    file: &FileWork,
    source_size: u64,
    staged: &str,
    evidence: &HostEvidence,
) -> io::Result<Option<Proof>> {
    if evidence.path != staged {
        return Err(failure("Hostbeleg der Stage passt nicht zur Publikation"));
    }
    let expected = with_backend(engine.view.source, |backend| {
        backend.read_size(&file.source, source_size)
    })?;
    if expected.is_some_and(|size| size != evidence.size) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Uploadziel ist keine reguläre Datei der übertragenen Größe: {staged}"),
        ));
    }
    let stage = evidence.proof();
    Ok((!engine.view.target.is_local() || stage.object.is_some()).then_some(stage))
}

pub(super) fn after(engine: &Engine<'_>, file: &FileWork, destination: &str) -> io::Result<()> {
    after_checked(engine, file, destination, None, None)
}

/// `after` for a local publication whose stage handle is still open: the
/// target is compared through it (M4), as the single file's observer does.
pub(super) fn after_held(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
    held: &std::fs::File,
) -> io::Result<()> {
    after_checked(engine, file, destination, Some(held), None)
}

/// `after` with the published target as the host's reply described it: no
/// stat or identity request of the target follows.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn after_known(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
    target: Option<&HostEvidence>,
) -> io::Result<()> {
    after_checked(engine, file, destination, None, target)
}

fn after_checked(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
    held: Option<&std::fs::File>,
    known: Option<&HostEvidence>,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    let mut receipt = record
        .read_file(file)?
        .ok_or_else(|| failure("Dateipublikation hat keinen vorbereiteten Beleg"))?;
    let mut confirmed_stamp = None;
    if let Some(proof) = receipt
        .proof
        .as_ref()
        .filter(|proof| proof.sha256.is_empty())
    {
        // A local publication's evidence is its stage's object, renamed into
        // place (at the receipt's destination): compared by metadata, and
        // that look is the new stamp. Copy bound it to the name (and flushed
        // it for moves and overwrites) before this hook. A local target whose
        // handle is still held is compared through it: one look at the path
        // (M4).
        confirmed_stamp = Some(match held.filter(|_| engine.view.target.is_local()) {
            Some(held) => observer::confirm_held(proof, held)?,
            None => super::file_evidence::confirm_stamp(proof, engine.view.target)?,
        });
    } else if let Some(proof) = &receipt.proof {
        let held = held.filter(|_| {
            engine.is_move() || engine.view.conflict == crate::types::Conflict::Overwrite
        });
        if let (Some(held), Some(identity)) = (held, proof.object.as_deref()) {
            crate::copy::confirm_transfer_publication_handle(
                &super::super::walk_listers::native(destination),
                held,
                identity,
                &proof.sha256,
                engine.stop_flag(),
            )?;
        } else {
            verify(proof, engine.view.target, engine.stop_flag(), false)?;
        }
    }
    // The target as acknowledged, by metadata only: a resumed job keeps it
    // without hashing while it is unchanged.
    let published = if receipt.proof.is_some() {
        match confirmed_stamp {
            Some(stamp) => stamp,
            None => super::file_evidence::stamp(engine.view.target, destination)?,
        }
    } else {
        // The target as the host's reply described it, or one look at it.
        let (known_stamp, meta) = match known {
            Some(evidence) => (
                Some(known_target(engine, file, &receipt, destination, evidence)?),
                None,
            ),
            None => (
                None,
                Some(super::file_evidence::validate_copy_target(
                    engine.view.source,
                    &file.source,
                    receipt.source.size,
                    engine.view.target,
                    destination,
                )?),
            ),
        };
        let (relative, destination) = destination_parts(&super::lock(&record.data), destination)?;
        receipt.destination = destination;
        receipt.relative = relative;
        receipt.previous = None;
        // A move reads from its quarantined source; the original path is
        // empty by then.
        match &receipt.quarantine {
            Some(quarantine) => {
                doubt::quarantined_source_unchanged(&receipt, quarantine, engine.view.source)?
            }
            None => doubt::source_unchanged(&receipt, engine.view.source)?,
        }
        match (known_stamp, meta) {
            (Some(stamp), _) => stamp,
            (None, Some(meta)) => {
                super::file_evidence::stamp_of(engine.view.target, &receipt.destination, meta)?
            }
            (None, None) => return Err(failure("Zielbeleg der Publikation fehlt")),
        }
    };
    receipt.confirmed = receipt
        .proof
        .as_ref()
        .is_some_and(|proof| proof.object.is_some());
    receipt.published = Some(Proof {
        path: receipt.destination.clone(),
        ..published
    });
    receipt.acknowledged = true;
    record.write_published(&receipt)
}

/// The published target as the host described it, checked like
/// `validate_copy_target`.
fn known_target(
    engine: &Engine<'_>,
    file: &FileWork,
    receipt: &Receipt,
    destination: &str,
    evidence: &HostEvidence,
) -> io::Result<Proof> {
    if evidence.path != destination {
        return Err(failure("Hostbeleg des Ziels passt nicht zur Publikation"));
    }
    let expected = with_backend(engine.view.source, |backend| {
        backend.read_size(&file.source, receipt.source.size)
    })?;
    if expected.is_some_and(|size| size != evidence.size) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Uploadziel ist keine reguläre Datei der übertragenen Größe: {destination}"),
        ));
    }
    Ok(evidence.proof())
}

pub(super) fn prepare_batch(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    prepare_direct(engine, file, &engine.folders.path_of(&file.rel))
}

pub(super) fn prepare_direct(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
) -> io::Result<()> {
    prepare_direct_known(engine, file, destination, None, None)
}

/// `prepare_direct` with the remote source and the destination's previous
/// presence as a host reply described them.
pub(super) fn prepare_direct_known(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
    source: Option<&HostEvidence>,
    existed_before: Option<bool>,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    active(engine)?;
    let source = source_guard(engine, file, source)?;
    if !source.verified || source.directory || source.link {
        return Err(failure("Paketquelle des Dateibelegs ist unbestätigt"));
    }
    record.start_file(
        file,
        source,
        None,
        destination.to_string(),
        None,
        previous_known(engine.view.target, destination, existed_before)?,
    )?;
    active(engine)
}

/// Whether `file` has a receipt in this job (memory only; no journal: no).
pub(super) fn has_receipt(engine: &Engine<'_>, file: &FileWork) -> bool {
    engine
        .job_record
        .as_ref()
        .is_some_and(|record| record.has_receipt(file))
}

pub(super) fn unpublished(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    let Some(mut receipt) = record.read_file(file)? else {
        return Ok(());
    };
    if let Some(quarantine) = &receipt.quarantine {
        match with_backend(engine.view.source, |backend| backend.stat(&quarantine.path)) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    receipt.discarded = true;
    receipt.proof = None;
    receipt.staged = None;
    receipt.quarantine = None;
    record.write_file(&receipt)
}

pub(super) fn preparation_failed(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    revoke(engine, file, None)
}

fn revoke(
    engine: &Engine<'_>,
    file: &FileWork,
    committed: Option<(&Path, &Path)>,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    if let Some(mut receipt) = record.read_file(file)? {
        if let Some((stage, target)) = committed {
            let stage = crate::copy::native_path_text(stage)?;
            let target = crate::copy::native_path_text(target)?;
            if receipt.confirmed
                || receipt.discarded
                || receipt.destination != target
                || receipt
                    .staged
                    .as_ref()
                    .map_or(true, |proof| proof.path != stage)
            {
                return Err(failure(
                    "Fehlgeschlagene Publikation passt nicht zum unbestätigten Stage-/Zielbeleg",
                ));
            }
        }
        receipt.publication_authorized = false;
        receipt.confirmed = false;
        receipt.acknowledged = false;
        record.write_file(&receipt)?;
    } else if committed.is_some() {
        return Err(failure(
            "Fehlgeschlagene Publikation hat keinen vorbereiteten Dateibeleg",
        ));
    }
    Ok(())
}
