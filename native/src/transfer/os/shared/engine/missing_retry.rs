//! Live "transfer missing" retains the journal without claiming crash ownership.
use super::{failure, verify, verify_source, with_backend, Receipt};
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
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let proof = receipt
                .proof
                .as_ref()
                .ok_or_else(|| failure("Bestätigtes Moveziel hat keinen Inhaltsbeleg"))?;
            verify(proof, view.target, cancel, false)?;
        }
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
    if live(&engine.view) && (receipt.acknowledged || receipt.confirmed) {
        return Ok(None);
    }
    let result = (|| {
        let proof = receipt
            .proof
            .as_ref()
            .ok_or_else(|| failure("Dateipublikation bleibt unbestätigt"))?;
        verify(proof, engine.view.target, engine.stop_flag(), true)?;
        verify_source(&receipt, engine.view.source, engine.stop_flag())?;
        Ok(())
    })();
    result.map_err(|error| OpError::at(At::Unknown, error))?;
    if crate::transfer::engine_names::parent_rel(&file.rel).is_none() {
        engine.folders.record_alias(
            &file.rel,
            crate::transfer::engine_names::base_name(&receipt.destination),
        );
    }
    Ok(Some(Outcome::Skipped))
}

#[cfg(test)]
#[path = "batches_missing_retry_compatibility_task_tests.rs"]
mod tests;
