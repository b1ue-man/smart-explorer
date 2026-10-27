//! Validate every saved publication before any source or stage recovery.
use std::io;

use super::super::file_evidence::{current_source, stamp_matches};
use super::state::{publication_state, Evidence};
use super::{
    doubt, endpoint, failure, missing_retry, receipt_path, verify, visit_saved, with_backend,
    write_record, AtomicBool, FileRoots, JobRecord, JobView, Ordering, Path, PreviousTarget, Proof,
    Receipt, RecordContext, RecordKind, Side,
};

/// What was at a destination before a publication that records no content
/// proof: presence only, never read. A resumed job decides by content
/// (`doubt`), for which the previous file's content is not needed.
pub(super) fn previous_presence(side: Side<'_>, path: &str) -> io::Result<PreviousTarget> {
    match with_backend(side, |backend| backend.stat(path)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PreviousTarget::Absent),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(PreviousTarget::Unconfirmed)
        }
        Err(error) => Err(error),
        Ok(_) => Ok(PreviousTarget::Unconfirmed),
    }
}

pub(super) fn selected_key(record: &JobRecord, receipt: &Receipt) -> String {
    match &record.items {
        crate::transfer::JobItems::Pairs(_) => receipt.key.clone(),
        _ => receipt.source.path.clone(),
    }
}

pub(in crate::transfer::engine) fn verify_saved(
    view: &JobView<'_>,
    record: &JobRecord,
    job_path: &Path,
    context: Option<&RecordContext>,
    cancel: &AtomicBool,
) -> io::Result<FileRoots> {
    if endpoint(view.source) != record.source || endpoint(view.target) != record.target {
        return Err(failure(
            "Dateiwiederaufnahme hat eine abweichende Quellen-/Zielverbindung",
        ));
    }
    let mut roots = FileRoots::new();
    let selected: std::collections::HashSet<&str> = record
        .guards
        .iter()
        .filter(|guard| !guard.directory)
        .map(|guard| guard.path.as_str())
        .collect();
    let mut recovery = Vec::new();
    // Validate every receipt before any quarantine cleanup or restoration.
    visit_saved(record, job_path, context, |receipt| {
        if receipt.discarded {
            return Ok(());
        }
        if missing_retry::acknowledged(view, record, &receipt, cancel)? {
            if record.moving && selected.contains(receipt.source.path.as_str()) {
                let key = selected_key(record, &receipt);
                if roots
                    .insert(
                        key.clone(),
                        (
                            receipt.source.clone(),
                            crate::transfer::ResolvedRoot {
                                source: key,
                                rel: receipt.relative.clone(),
                            },
                        ),
                    )
                    .is_some()
                {
                    return Err(failure("Mehrdeutige Dateizielbelege in der Auswahl"));
                }
            }
            return Ok(());
        }
        let evidence = publication_state(&receipt, view, cancel)?;
        let published = evidence.is_some();
        let quarantine_exists = match &receipt.quarantine {
            Some(quarantine) => {
                match with_backend(view.source, |backend| backend.stat(&quarantine.path)) {
                    Ok(_) => {
                        verify(quarantine, view.source, cancel, true)?;
                        true
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                    Err(error) => return Err(error),
                }
            }
            None => false,
        };
        match with_backend(view.source, |backend| backend.stat(&receipt.source.path)) {
            Ok(_) => verify_source(&receipt, view.source, cancel)?,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && record.moving
                    && (published || quarantine_exists) => {}
            Err(error) => return Err(error),
        }
        if !published {
            if let Some(stage) = receipt.staged.as_ref().filter(|_| view.target.is_local()) {
                match with_backend(view.target, |backend| backend.stat(&stage.path)) {
                    Ok(_) => verify(stage, view.target, cancel, true)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        } else if selected.contains(receipt.source.path.as_str()) {
            let key = selected_key(record, &receipt);
            if roots
                .insert(
                    key.clone(),
                    (
                        receipt.source.clone(),
                        crate::transfer::ResolvedRoot {
                            source: key,
                            rel: receipt.relative.clone(),
                        },
                    ),
                )
                .is_some()
            {
                return Err(failure("Mehrdeutige Dateizielbelege in der Auswahl"));
            }
        }
        // A remote stage left by a crash goes in any case: published or not,
        // the job never publishes it again.
        let remote_stage = !view.target.is_local() && receipt.staged.is_some();
        if quarantine_exists || !published || remote_stage {
            recovery.push((receipt, evidence, quarantine_exists));
        }
        Ok(())
    })?;
    for (receipt, evidence, quarantine_exists) in recovery {
        let published = evidence.is_some();
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Dateiwiederaufnahme abgebrochen",
            ));
        }
        if quarantine_exists {
            if !view.source.is_local() {
                return Err(failure("Movequarantäne ist nicht lokal"));
            }
            let quarantine = receipt
                .quarantine
                .as_ref()
                .ok_or_else(|| failure("Movequarantäne fehlt"))?;
            let identity = quarantine
                .object
                .as_deref()
                .ok_or_else(|| failure("Movequarantäne ohne Objektidentität"))?;
            if let Some(evidence) = &evidence {
                confirm_move_target(&receipt, evidence, view, cancel)?;
                let root = with_backend(view.target, |backend| {
                    crate::vfs::directory_identity(backend, &record.target_dir)
                })?;
                if root.is_none() || root != record.target_object {
                    return Err(failure(
                        "Movezielroot während der Dauerhaftigkeitsbestätigung geändert",
                    ));
                }
            }
            crate::copy::recover_transfer_quarantine(
                &crate::transfer::walk_listers::native(&receipt.source.path),
                &crate::transfer::walk_listers::native(&quarantine.path),
                identity,
                published,
            )?;
        }
        if let Some(stage) = &receipt.staged {
            if !published || !view.target.is_local() {
                discard_stage(stage, view.target)?;
            }
        }
        if !published {
            verify_source(&receipt, view.source, cancel)?;
            let mut receipt = receipt;
            receipt.discarded = true;
            receipt.proof = None;
            receipt.staged = None;
            receipt.quarantine = None;
            write_record(
                &receipt_path(job_path, &receipt.key),
                RecordKind::Job,
                &record.id,
                context,
                &receipt,
            )?;
        }
    }
    Ok(roots)
}

/// Binds a move's published target before its quarantined source goes:
/// by its object (unchanged metadata decided), or by object and content
/// (the digest decided, or a content proof of 0.5.185/0.5.186).
fn confirm_move_target(
    receipt: &Receipt,
    evidence: &Evidence,
    view: &JobView<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !view.target.is_local() {
        return Err(failure("Moveziel ist nicht lokal"));
    }
    let target = crate::transfer::walk_listers::native(&receipt.destination);
    let identity = evidence
        .identity
        .as_deref()
        .ok_or_else(|| failure("Moveziel ohne Objektidentität"))?;
    // Only the durable positive receipt carries the successful concrete
    // platform operation across a restart. An authorized but unacknowledged
    // crash still needs a fresh parent proof.
    let acknowledged = receipt.confirmed && receipt.acknowledged && receipt.publication_authorized;
    let Some(sha256) = &evidence.sha256 else {
        return crate::copy::confirm_transfer_publication_identity(
            &target,
            identity,
            acknowledged,
            cancel,
        );
    };
    if acknowledged {
        crate::copy::confirm_acknowledged_transfer_publication(&target, identity, sha256, cancel)
    } else {
        crate::copy::confirm_transfer_publication(&target, identity, sha256, cancel)
    }
}

/// Removes a stage a crash left: a local one through its object identity, a
/// remote one only while it still has its recorded metadata (anything else
/// there is not provably this job's and stays).
fn discard_stage(stage: &Proof, side: Side<'_>) -> io::Result<()> {
    match with_backend(side, |backend| backend.stat(&stage.path)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) if side.is_local() => crate::copy::discard_transfer_stage(
            &crate::transfer::walk_listers::native(&stage.path),
            stage
                .object
                .as_deref()
                .ok_or_else(|| failure("Private Stage ohne Objektidentität"))?,
        ),
        Ok(_) if !stamp_matches(stage, side)? => Ok(()),
        Ok(_) => match with_backend(side, |backend| backend.discard_copy_stage(&stage.path)) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(())
            }
            result => result,
        },
    }
}

/// The source is still the recorded file. Its content is compared only
/// against a stored content proof (receipts of 0.5.185/0.5.186), hashed on
/// its device where possible; otherwise metadata decides.
pub(super) fn verify_source(
    receipt: &Receipt,
    side: Side<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let Some(expected) = [receipt.proof.as_ref(), receipt.quarantine.as_ref()]
        .into_iter()
        .flatten()
        .map(|proof| proof.sha256.as_str())
        .find(|sha256| !sha256.is_empty())
    else {
        return doubt::source_unchanged(receipt, side);
    };
    let after = current_source(
        side,
        &receipt.source.path,
        receipt.source.id.as_deref(),
        cancel,
    )?;
    if after.id != receipt.source.id
        || after.object != receipt.source.object
        || after.size != receipt.source.size
        || after.mtime_ms != receipt.source.mtime_ms
    {
        return Err(failure(format!(
            "Quelle des Dateibelegs wurde geändert: {}",
            receipt.source.path
        )));
    }
    if after.sha256 != expected {
        return Err(failure(format!(
            "Quellinhalt des Dateibelegs wurde geändert: {}",
            receipt.source.path
        )));
    }
    Ok(())
}
