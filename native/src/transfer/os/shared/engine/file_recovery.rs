//! Per-file publication receipts. A private stage's object identity and
//! content digest are recorded durably before publication; equal size never
//! establishes ownership during recovery.
use super::file_evidence::{
    observe, observe_available, observe_source, readback_unavailable, verify, Proof,
};
use super::job_store::{endpoint, snapshot, with_backend, JobRun};
use super::queue::FileWork;
use super::record_io::{read_record, write_record, RecordContext, RecordKind};
use super::view::{JobView, Side};
use super::Engine;
use crate::transfer::job_record::{JobRecord, SourceGuard, StoredEndpoint};
#[cfg(test)]
#[path = "batches_publication_compatibility_task_tests.rs"]
mod compatibility_tests;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "missing_retry.rs"]
pub(super) mod missing_retry;
#[path = "file_recovery_observer.rs"]
mod observer;
#[path = "file_recovery_paths.rs"]
mod paths;
#[path = "file_recovery_receipts.rs"]
mod receipts;
#[path = "file_recovery_verify.rs"]
mod verification;

pub(super) use observer::LocalObserver;
use paths::destination_parts;
pub(super) use receipts::initialize;
use receipts::{
    read_index, receipt_path, repair_new_entry, visit_saved, PreviousTarget, Receipt,
};
#[cfg(test)]
use verification::publication_state;
pub(super) use verification::verify_saved;
use verification::{previous_target, verify_source};

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
    let proof = observe_available(engine.view.target, staged, engine.stop_flag())?;
    if proof.is_none() {
        super::file_evidence::validate_copy_target(
            engine.view.source,
            &file.source,
            source.size,
            engine.view.target,
            staged,
        )?;
    }
    record.start_file(
        file,
        source,
        proof,
        destination.to_string(),
        None,
        previous_target(engine.view.target, destination, engine.stop_flag())?,
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
    let mut observed = true;
    if let Some(proof) = &receipt.proof {
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
            match verify(proof, engine.view.target, engine.stop_flag(), false) {
                Ok(()) => {}
                Err(error) if readback_unavailable(engine.view.target, &error) => {
                    super::file_evidence::validate_target_shape(
                        engine.view.target,
                        destination,
                        Some(proof.size),
                    )?;
                    observed = false;
                }
                Err(error) => return Err(error),
            }
        }
    } else {
        let proof = observe_available(engine.view.target, destination, engine.stop_flag())?;
        if proof.is_none() {
            super::file_evidence::validate_copy_target(
                engine.view.source,
                &file.source,
                receipt.source.size,
                engine.view.target,
                destination,
            )?;
        }
        let (relative, destination) = destination_parts(&super::lock(&record.data), destination)?;
        receipt.destination = destination;
        receipt.relative = relative;
        receipt.previous = None;
        receipt.proof = proof;
        verify_source(&receipt, engine.view.source, engine.stop_flag())?;
    }
    receipt.confirmed = observed
        && receipt
            .proof
            .as_ref()
            .is_some_and(|proof| proof.object.is_some());
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
        previous_target(engine.view.target, destination, engine.stop_flag())?,
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
