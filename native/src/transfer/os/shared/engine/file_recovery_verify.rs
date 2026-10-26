//! Validate every saved publication before any source or stage recovery.
use std::io;

use super::{
    endpoint, failure, missing_retry, observe, observe_source, receipt_path, verify,
    visit_saved, with_backend, write_record, AtomicBool, FileRoots, JobRecord,
    JobView, Ordering, Path, PreviousTarget, Receipt, RecordContext, RecordKind, Side,
};

pub(super) fn previous_target(side: Side<'_>, path: &str, cancel: &AtomicBool) -> io::Result<PreviousTarget> {
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
        Ok(meta) if meta.is_dir || meta.is_symlink || meta.special => {
            Ok(PreviousTarget::Unconfirmed)
        }
        Ok(_) => match observe(side, path, cancel) {
            Ok(proof) => Ok(PreviousTarget::File(proof)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(PreviousTarget::Unconfirmed)
            }
            Err(error) => Err(error),
        },
    }
}

pub(super) fn previous_matches(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<bool> {
    match &receipt.previous {
        Some(PreviousTarget::Absent) => {
            match with_backend(side, |backend| backend.stat(&receipt.destination)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
                Err(error) => Err(error),
                Ok(_) => Ok(false),
            }
        }
        Some(PreviousTarget::File(proof)) => match verify(proof, side, cancel, true) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(_) => Ok(false),
        },
        _ => Ok(false),
    }
}

pub(super) fn publication_state(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<bool> {
    // A pending new-key intent, failed preparation, or live-proven failed
    // publication can write this durable negative acknowledgement.
    if !receipt.publication_authorized {
        return Ok(false);
    }
    let Some(proof) = &receipt.proof else {
        return if receipt.quarantine.is_some() {
            Ok(false)
        } else {
            Err(failure(format!(
                "Veröffentlichungsergebnis bleibt unbestätigt: {}",
                receipt.destination
            )))
        };
    };
    if receipt.publication_authorized && proof.object.is_none() {
        return Err(failure(format!(
            "Dateieigentum ohne stabile Objektidentität ist unbestätigt: {}",
            proof.path
        )));
    }
    match verify(proof, side, cancel, true) {
        Ok(()) if receipt.publication_authorized => return Ok(true),
        Err(error) if receipt.confirmed || error.kind() == io::ErrorKind::Interrupted => {
            return Err(error)
        }
        _ => {}
    }
    let stage = receipt
        .staged
        .as_ref()
        .ok_or_else(|| failure("Vorpublikationszustand ohne eigene Stage"))?;
    let stage_present = match with_backend(side, |backend| backend.stat(&stage.path)) {
        Ok(_) => {
            verify(stage, side, cancel, true)?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if (!stage_present && receipt.publication_authorized)
        || !previous_matches(receipt, side, cancel)?
    {
        return Err(failure(format!(
            "Zielersatz oder unbestätigte Publikationsphase: {}",
            receipt.destination
        )));
    }
    Ok(false)
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
        let published = publication_state(&receipt, view.target, cancel)?;
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
            if let Some(stage) = &receipt.staged {
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
        if quarantine_exists || !published {
            recovery.push((receipt, published, quarantine_exists));
        }
        Ok(())
    })?;
    for (receipt, published, quarantine_exists) in recovery {
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
            if published {
                let proof = receipt
                    .proof
                    .as_ref()
                    .ok_or_else(|| failure("Publiziertes Moveziel ohne Beleg"))?;
                if !view.target.is_local() {
                    return Err(failure("Moveziel ist nicht lokal"));
                }
                crate::copy::confirm_transfer_publication(
                    &crate::transfer::walk_listers::native(&proof.path),
                    proof
                        .object
                        .as_deref()
                        .ok_or_else(|| failure("Moveziel ohne Objektidentität"))?,
                    &proof.sha256,
                    cancel,
                )?;
                verify(proof, view.target, cancel, true)?;
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
        if !published {
            verify_source(&receipt, view.source, cancel)?;
            if let Some(stage) = &receipt.staged {
                match with_backend(view.target, |backend| backend.stat(&stage.path)) {
                    Ok(_) if view.target.is_local() => crate::copy::discard_transfer_stage(&crate::transfer::walk_listers::native(&stage.path), stage.object.as_deref().ok_or_else(|| failure("Private Stage ohne Objektidentität"))?)?,
                    Ok(_) => return Err(failure(format!("Private Remotestage bleibt bis zur bestätigten providergebundenen Bereinigung erhalten: {}", stage.path))),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                    Err(error) => return Err(error),
                }
            }
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

pub(super) fn verify_source(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<()> {
    let after = observe_source(
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
    if let Some(proof) = receipt.proof.as_ref().or(receipt.quarantine.as_ref()) {
        if after.sha256 != proof.sha256 {
            return Err(failure(format!(
                "Quellinhalt des Dateibelegs wurde geändert: {}",
                receipt.source.path
            )));
        }
    }
    Ok(())
}
