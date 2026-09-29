//! One checked content choice leaves at most one exact object on each side.
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use crate::vfs::Backend;
use super::duplicate_observation::{changed, check_cancel, observe, read_content, verify, verify_content};
use super::duplicate_types::{DuplicateConflict, FileVariant};
use super::incremental::SyncEndpoints;
use super::paths::{join, parent_of};
use super::resolve_conflict::ResolvePhase;
use super::types::{Conflict, Sig, Throttle};

fn choose(group: &DuplicateConflict, keep_a: bool, id: Option<&str>) -> io::Result<Option<FileVariant>> {
    let variants = group.variants(keep_a);
    if let Some(id) = id {
        return variants.iter().find(|v| v.id.as_deref() == Some(id)).cloned()
            .map(Some).ok_or_else(changed);
    }
    if group.needs_variant_choice(keep_a) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "Auf der gewählten Seite liegen verschiedene Dateien mit demselben Namen. Bitte eine konkrete Version auswählen."));
    }
    Ok(variants.first().cloned())
}

struct Side<'a> {
    backend: &'a dyn Backend,
    path: String,
    files: Vec<FileVariant>,
    keep: Option<FileVariant>,
}

impl Side<'_> {
    fn verify(&self, cancel: &AtomicBool) -> io::Result<()> {
        verify(self.backend, &self.path, &self.files, cancel)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve(
    endpoints: SyncEndpoints<'_>, conflict: &Conflict, keep_a: bool, id: Option<&str>,
    versions: &Path, cancel: &AtomicBool, bandwidth_bps: u64, mut progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    check_cancel(cancel)?;
    crate::vfs::validate_sync_roots(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)?;
    let group = conflict.duplicates.as_ref().ok_or_else(changed)?;
    let selected = choose(group, keep_a, id)?;
    let flows = super::sync_flows::PairFlows::new(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let _permit = flows.transfer(cancel).ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "Konfliktauflösung abgebrochen"))?;
    let throttle = Throttle::new(bandwidth_bps);
    let mut a = Side { backend: endpoints.a, path: join(endpoints.root_a, &conflict.rel),
        files: group.a.clone(), keep: None };
    let mut b = Side { backend: endpoints.b, path: join(endpoints.root_b, &conflict.rel),
        files: group.b.clone(), keep: None };
    progress(ResolvePhase::Preparing);
    a.verify(cancel)?;
    b.verify(cancel)?;
    progress(ResolvePhase::BackingUp);
    for (label, side) in [("A", &a), ("B", &b)] {
        for variant in &side.files {
            super::duplicate_backup::save(side.backend, &side.path, &conflict.rel, versions, variant, cancel, &throttle)
                .map_err(|e| context(e, &format!("Sicherung von Seite {label} fehlgeschlagen; die Dateivarianten wurden nicht verändert")))?;
        }
    }
    a.verify(cancel)?;
    b.verify(cancel)?;
    if let Some(selected) = selected.as_ref() {
        let (source, destination) = if keep_a { (&mut a, &mut b) } else { (&mut b, &mut a) };
        source.keep = Some(selected.clone());
        destination.keep = destination.files.iter().find(|v| selected.same_content(v)).cloned();
        if destination.keep.is_none() {
            progress(ResolvePhase::Copying);
            publish(source, destination, selected, cancel, &throttle)
                .map_err(|e| context(e, "Übernahme der gewählten Version nicht bestätigt; bitte erneut vergleichen"))?;
        }
    }
    progress(ResolvePhase::Deleting);
    // Every removal verifies both groups, including the survivor, immediately
    // beforehand. A partial failure remains retryable, with all bytes backed up.
    remove_extras(&mut a, &b, cancel)
        .map_err(|e| context(e, "Bereinigung auf Seite A unvollständig; Sicherungen bleiben erhalten, bitte erneut vergleichen"))?;
    remove_extras(&mut b, &a, cancel)
        .map_err(|e| context(e, "Bereinigung auf Seite B unvollständig; Sicherungen bleiben erhalten, bitte erneut vergleichen"))?;
    progress(ResolvePhase::ReadingSignatures);
    // After the last commit, finish verification even if Stop was just pressed.
    let finish = AtomicBool::new(false);
    a.verify(&finish)?;
    b.verify(&finish)?;
    if a.files.len() > 1 || b.files.len() > 1 { return Err(changed()); }
    match selected {
        Some(ref selected) if !a.files.first().is_some_and(|v| selected.same_content(v))
            || !b.files.first().is_some_and(|v| selected.same_content(v)) => return Err(changed()),
        None if !a.files.is_empty() || !b.files.is_empty() => return Err(changed()),
        _ => {}
    }
    Ok((a.files.first().map(|v| v.signature), b.files.first().map(|v| v.signature)))
}

fn context(error: io::Error, operation: &str) -> io::Error {
    io::Error::new(error.kind(), format!("{operation}: {error}"))
}

fn publish(source: &Side<'_>, destination: &mut Side<'_>, selected: &FileVariant,
    cancel: &AtomicBool, throttle: &Throttle) -> io::Result<()> {
    if let Some(parent) = parent_of(&destination.path) { destination.backend.mkdir_all(&parent)?; }
    let staged = crate::vfs::unique_staging_path(destination.backend, &destination.path, "bisync-variant")?;
    let result = (|| {
        let mut writer = destination.backend.open_write_copy_stage_sized(&staged, selected.content_size)?;
        verify_content(read_content(source.backend, &source.path, selected.id.as_deref(), &mut *writer, cancel, Some(throttle))?, selected)?;
        writer.flush()?;
        drop(writer);
        let staged_files = observe(destination.backend, &staged, None, cancel)?;
        if staged_files.len() != 1 || !selected.same_content(&staged_files[0]) { return Err(changed()); }
        source.verify(cancel)?;
        destination.verify(cancel)?;
        check_cancel(cancel)?;
        let replaced = destination.files.first();
        match replaced {
            Some(file) => destination.backend.promote_staged_to_id(&staged, &destination.path, file.id.as_deref())?,
            None => crate::vfs::promote_staged_create(destination.backend, &staged, &destination.path)?,
        }
        let finish = AtomicBool::new(false);
        let after = observe(destination.backend, &destination.path, None, &finish)?;
        let keep = match replaced {
            Some(file) => after.iter().find(|v| v.id == file.id),
            None => after.first().filter(|_| after.len() == 1),
        }.filter(|v| selected.same_content(v)).cloned().ok_or_else(changed)?;
        let old_others: Vec<_> = destination.files.iter().filter(|v| Some(*v) != replaced).collect();
        let new_others: Vec<_> = after.iter().filter(|v| v.id != keep.id).collect();
        if old_others != new_others { return Err(changed()); }
        destination.keep = Some(keep);
        destination.files = after;
        Ok(())
    })();
    if result.is_err() { let _ = destination.backend.discard_copy_stage(&staged); }
    result
}

fn remove_extras(side: &mut Side<'_>, other: &Side<'_>, cancel: &AtomicBool) -> io::Result<()> {
    let extras: Vec<_> = side.files.iter().filter(|v| Some(*v) != side.keep.as_ref()).cloned().collect();
    for extra in extras {
        check_cancel(cancel)?;
        side.verify(cancel)?;
        other.verify(cancel)?;
        check_cancel(cancel)?;
        side.backend.remove_file_id(&side.path, extra.id.as_deref())?;
        side.files.retain(|v| v != &extra);
        let finish = AtomicBool::new(false);
        side.verify(&finish)?;
    }
    Ok(())
}
