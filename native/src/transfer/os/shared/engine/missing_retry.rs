//! Live "transfer missing" retains the journal without claiming crash
//! ownership; resumed and retried files are decided in doubt per file.
use super::doubt::{self, Ownership};
use super::{failure, verify, verify_source, with_backend, JobRun, Receipt};
use crate::transfer::engine::file_evidence::{decisive, readback_unavailable, stamp_matches};
use crate::transfer::engine::view::JobView;
use crate::transfer::engine::{
    ops::{At, OpError, Outcome},
    queue::FileWork,
    Engine,
};
use crate::transfer::job_record::JobRecord;
use std::{io, path::Path, sync::atomic::AtomicBool};

pub(crate) fn live(view: &JobView<'_>) -> bool {
    view.resume.is_some() && view.recovery == crate::transfer::RecoveryMode::Resume
}

/// Every protected mutation follows the durable job and at least one intent.
/// A live failure before begin has neither a result mapping nor any journal.
/// Stored recovery never uses this exception or defaults a missing index.
pub(crate) fn unstarted(view: &JobView<'_>, job_path: &Path) -> io::Result<bool> {
    if !live(view) || view.resume.is_none_or(|roots| !roots.is_empty()) {
        return Ok(false);
    }
    let Some(id) = view.id else {
        return Ok(false);
    };
    let aliases = crate::support_dirs::app_data_file("transfer-aliases").join(format!("{id}.json"));
    for path in [job_path.with_extension("files"), aliases] {
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

pub(crate) fn target(view: &JobView<'_>, record: &JobRecord) -> io::Result<bool> {
    if !live(view) || record.mutation_started && record.target_object.is_some() {
        return Ok(false);
    }
    with_backend(view.target, |backend| {
        match backend.stat(&record.target_dir) {
            Ok(meta) if meta.is_dir && !meta.is_symlink && !meta.special => Ok(true),
            Ok(_) => Err(failure(
                "Zielroot der aktuellen Wiederholung ist kein regulärer Ordner",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    })
}

/// This branch never removes a stage, target or quarantined source. An
/// unfinished move with a retained quarantine still needs strict recovery.
pub(super) fn acknowledged(
    view: &JobView<'_>,
    record: &JobRecord,
    receipt: &Receipt,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    if !live(view)
        || !receipt.publication_authorized
        || !(receipt.acknowledged || receipt.confirmed)
    {
        return Ok(false);
    }
    if !record.moving {
        return Ok(true);
    }
    if let Some(quarantine) = &receipt.quarantine {
        match with_backend(view.source, |backend| backend.stat(&quarantine.path)) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match with_backend(view.source, |backend| backend.stat(&receipt.source.path)) {
        Ok(_) => verify_source(receipt, view.source, cancel)?,
        // The live move removed its source only after the acknowledged
        // publication: the target is the only copy and is never touched.
        Err(error) if error.kind() == io::ErrorKind::NotFound => match &receipt.proof {
            Some(proof) if !proof.sha256.is_empty() => verify(proof, view.target, cancel, false)?,
            _ if doubt::shape(receipt, view.source, view.target)?.is_none() => {}
            _ => {
                return Err(failure(format!(
                    "Bestätigtes Moveziel fehlt oder weicht ab: {}",
                    receipt.destination
                )))
            }
        },
        Err(error) => return Err(error),
    }
    Ok(true)
}

pub(crate) fn resume(engine: &Engine<'_>, file: &FileWork) -> Result<Option<Outcome>, OpError> {
    let Some(record) = &engine.job_record else {
        return Ok(None);
    };
    let Some(receipt) = record
        .read_file(file)
        .map_err(|error| OpError::at(At::Unknown, error))?
    else {
        return Ok(None);
    };
    if receipt.discarded {
        return Ok(None);
    }
    if !receipt.publication_authorized {
        return Err(OpError::at(
            At::Unknown,
            failure("Journalneueintrag hat keine bestätigte Publikationserlaubnis"),
        ));
    }
    // Unchanged since this job acknowledged it: its own, nothing is read.
    match unchanged(engine, &receipt) {
        Ok(true) => return Ok(Some(own(engine, file, &receipt))),
        Ok(false) => {}
        Err(error) => return Err(OpError::at(At::Unknown, error)),
    }
    let content_proof = receipt.proof.clone().filter(|p| !p.sha256.is_empty());
    let Some(proof) = content_proof else {
        return in_doubt(engine, record, file, receipt);
    };
    // A content proof of 0.5.185/0.5.186.
    if live(&engine.view) && (receipt.acknowledged || receipt.confirmed) {
        return Ok(None);
    }
    let result = (|| {
        verify(&proof, engine.view.target, engine.stop_flag(), false)?;
        verify_source(&receipt, engine.view.source, engine.stop_flag())
    })();
    result.map_err(|error| OpError::at(At::Unknown, error))?;
    Ok(Some(own(engine, file, &receipt)))
}

/// The target still has the metadata identity recorded when it was
/// published (the acknowledgement's stamp, or a local stage's object renamed
/// into place) and the source is unchanged: decided without hashing.
fn unchanged(engine: &Engine<'_>, receipt: &Receipt) -> io::Result<bool> {
    let metadata = receipt.proof.as_ref().filter(|p| p.sha256.is_empty());
    let evidence = receipt.published.as_ref().or(metadata);
    let Some(evidence) = evidence.filter(|stamp| decisive(stamp)) else {
        return Ok(false);
    };
    Ok(stamp_matches(evidence, engine.view.target)?
        && doubt::source_unchanged(receipt, engine.view.source).is_ok())
}

/// A receipt without a stored content proof: the content digest of source
/// and target, each from the device that holds it, decides (`doubt`). Only
/// here, never in the live uninterrupted path.
fn in_doubt(
    engine: &Engine<'_>,
    record: &JobRun,
    file: &FileWork,
    mut receipt: Receipt,
) -> Result<Option<Outcome>, OpError> {
    let live = live(&engine.view);
    let decided = match doubt::ownership(
        &receipt,
        engine.view.source,
        engine.view.target,
        engine.stop_flag(),
    ) {
        Ok(decided) => decided,
        // "Transfer missing" on a target that can be neither hashed nor
        // read keeps its rule (K8): a same-sized file stays, nothing is
        // replaced. The job log says why.
        Err(error) if live && readback_unavailable(engine.view.target, &error) => {
            engine.issues.note(&receipt.destination, &error.to_string());
            return Ok(None);
        }
        Err(error) => return Err(OpError::at(At::Unknown, error)),
    };
    match decided {
        Ownership::Own => Ok(Some(own(engine, file, &receipt))),
        // "Transfer missing" never replaces an existing file (K8).
        Ownership::Foreign if live => Err(OpError::at(
            At::Publish,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "Ziel „{}“ hat einen anderen Inhalt als die Quelle – nicht ersetzt",
                    receipt.destination
                ),
            ),
        )),
        // Not this job's file: transferred like a fresh run would, under
        // the job's conflict policy (keep both, overwrite or skip).
        Ownership::Foreign | Ownership::Absent => {
            receipt.discarded = true;
            receipt.proof = None;
            receipt.staged = None;
            record
                .write_file(&receipt)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Ok(None)
        }
    }
}

/// The destination holds this job's file: skipped, under its actual name.
fn own(engine: &Engine<'_>, file: &FileWork, receipt: &Receipt) -> Outcome {
    if crate::transfer::engine_names::parent_rel(&file.rel).is_none() {
        engine.folders.record_alias(
            &file.rel,
            crate::transfer::engine_names::base_name(&receipt.destination),
        );
    }
    Outcome::Skipped
}

#[cfg(test)]
#[path = "batches_missing_retry_compatibility_task_tests.rs"]
mod tests;
