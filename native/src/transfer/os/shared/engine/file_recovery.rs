//! Per-file publication receipts. The live path records the source, the
//! destination and (for a local stage) the stage's identity before
//! publication, without reading any file back. A resumed or retried job
//! decides ownership in doubt by content digest (`doubt`); equal size alone
//! never establishes it. Local copies and moves keep their content proofs.
use super::file_evidence::{verify, Proof};
use super::job_store::{endpoint, snapshot, with_backend, JobRun};
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
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "file_recovery_doubt.rs"]
mod doubt;
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

pub(super) use observer::LocalObserver;
use paths::destination_parts;
pub(super) use receipts::initialize;
use receipts::{read_index, receipt_path, repair_new_entry, visit_saved, PreviousTarget, Receipt};
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

pub(super) fn before(
    engine: &Engine<'_>,
    file: &FileWork,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    active(engine)?;
    let source = snapshot(engine.view.source, &file.source);
    if !source.verified || source.directory || source.link {
        return Err(failure("Quelle des Dateibelegs ist unbestätigt"));
    }
    // No content proof by reading the file back (user 2026-10-27: far too
    // slow, it read every file back twice): the transport delivers every byte
    // integrity-protected and acknowledges each write; size and kind of the
    // stage are checked. A resumed job decides ownership by metadata, in
    // doubt by content digest (`doubt`); the stage is recorded by metadata
    // so a crash removes exactly it.
    let stage = super::file_evidence::validate_copy_target(
        engine.view.source,
        &file.source,
        source.size,
        engine.view.target,
        staged,
    )?;
    record.start_file(
        file,
        source,
        super::file_evidence::stage_record(engine.view.target, staged, stage)?,
        destination.to_string(),
        None,
        previous_presence(engine.view.target, destination)?,
    )?;
    active(engine)
}

pub(super) fn after(engine: &Engine<'_>, file: &FileWork, destination: &str) -> io::Result<()> {
    after_checked(engine, file, destination, None)
}

fn after_checked(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
    held: Option<&std::fs::File>,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    let mut receipt = record
        .read_file(file)?
        .ok_or_else(|| failure("Dateipublikation hat keinen vorbereiteten Beleg"))?;
    if let Some(proof) = receipt
        .proof
        .as_ref()
        .filter(|proof| proof.sha256.is_empty())
    {
        // A local publication's evidence is its stage's object, renamed into
        // place: compared by metadata. Copy bound it to the name (and flushed
        // it for moves and overwrites) before this hook.
        verify(proof, engine.view.target, engine.stop_flag(), false)?;
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
        super::file_evidence::stamp(engine.view.target, destination)?
    } else {
        let meta = super::file_evidence::validate_copy_target(
            engine.view.source,
            &file.source,
            receipt.source.size,
            engine.view.target,
            destination,
        )?;
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
        super::file_evidence::stamp_of(engine.view.target, &receipt.destination, meta)?
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
    record.write_file(&receipt)
}

pub(super) fn prepare_batch(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    prepare_direct(engine, file, &engine.folders.path_of(&file.rel))
}

pub(super) fn prepare_direct(
    engine: &Engine<'_>,
    file: &FileWork,
    destination: &str,
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    active(engine)?;
    let source = snapshot(engine.view.source, &file.source);
    if !source.verified || source.directory || source.link {
        return Err(failure("Paketquelle des Dateibelegs ist unbestätigt"));
    }
    record.start_file(
        file,
        source,
        None,
        destination.to_string(),
        None,
        previous_presence(engine.view.target, destination)?,
    )?;
    active(engine)
}

pub(super) fn unpublished(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    let receipt = {
        let data = super::lock(&record.data);
        let mut index = read_index(&record.path, &data, record.context.as_ref())?;
        repair_new_entry(&record.path, &data, record.context.as_ref(), &mut index)?;
        record.read_file_unlocked(file, &data)?
    };
    let Some(mut receipt) = receipt else {
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
    let data = super::lock(&record.data);
    let mut index = read_index(&record.path, &data, record.context.as_ref())?;
    repair_new_entry(&record.path, &data, record.context.as_ref(), &mut index)?;
    if let Some(mut receipt) = record.read_file_unlocked(file, &data)? {
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
