//! One checked content choice leaves at most one exact object on each side.
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use crate::vfs::Backend;
use super::apply_guard::{CapturedFile, ExpectedFile};
use super::checkpoint::{ApplyScope, CollectingSink};
use super::completion::{CompletedAction, CompletedKind};
use super::duplicate_observation::{changed, metadata_named, observe_named, verify_named};
use super::duplicate_types::{DuplicateConflict, FileVariant};
use super::incremental::SyncEndpoints;
use super::resolve_conflict::ResolvePhase;
use super::types::{BisyncOptions, Conflict, PairSide, Sig, Throttle};
use super::versions::{RunVersions, VersionSide, VersionsContext};

fn choose(group: &DuplicateConflict, keep_a: bool, id: Option<&str>) -> io::Result<Option<FileVariant>> {
    let variants = group.variants(keep_a);
    if let Some(id) = id {
        return variants.iter().find(|variant| variant.id.as_deref() == Some(id)).cloned()
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
    root: &'a str,
    cross_mounts: bool,
    side: PairSide,
    rel: String,
    path: String,
    files: Vec<FileVariant>,
    keep: Option<FileVariant>,
}
impl Side<'_> {
    fn name(&self) -> &str { self.rel.rsplit('/').next().unwrap_or(&self.rel) }
    fn verify(&self, cancel: &AtomicBool) -> io::Result<()> {
        super::apply_boundary::guard(self.backend, self.root, &self.rel, self.cross_mounts)?;
        verify_named(self.backend, &self.path, self.name(), &self.files, cancel)
    }
    fn version_side(&self) -> VersionSide<'_> { VersionSide { side: self.side, backend: self.backend, root: self.root } }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve(endpoints: SyncEndpoints<'_>, conflict: &Conflict, keep_a: bool, id: Option<&str>,
    versions: &Path, cancel: &AtomicBool, bandwidth_bps: u64, progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    let pair = super::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let context = VersionsContext::new(&pair, super::StateOwner::AdHoc, super::VersionsLocation::AppData, Default::default());
    let versions = RunVersions::with_app_data(context, versions.to_path_buf());
    let sink = CollectingSink::default();
    let spellings = super::Spellings::default();
    let scope = ApplyScope { sink: &sink, versions: &versions, spellings: &spellings };
    resolve_scoped(endpoints, conflict, keep_a, id, BisyncOptions::default(), &scope, cancel, bandwidth_bps, progress)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_scoped(endpoints: SyncEndpoints<'_>, conflict: &Conflict, keep_a: bool,
    id: Option<&str>, opts: BisyncOptions, scope: &ApplyScope<'_>, cancel: &AtomicBool,
    bandwidth_bps: u64, mut progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    super::apply_transaction::gate(cancel, Some(scope.sink))?;
    crate::vfs::validate_sync_roots(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)?;
    scope.versions.bind_lock(&super::pair_lock_id(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b))?;
    let group = conflict.duplicates.as_ref().ok_or_else(changed)?;
    let selected = choose(group, keep_a, id)?;
    let flows = super::sync_flows::PairFlows::new(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let _permit = flows.transfer(cancel).ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "Konfliktauflösung abgebrochen"))?;
    let throttle = Throttle::new(bandwidth_bps);
    let rel_a = scope.spellings.side_rel(&conflict.rel, PairSide::A).to_string();
    let rel_b = scope.spellings.side_rel(&conflict.rel, PairSide::B).to_string();
    let mut a = Side { backend: endpoints.a, root: endpoints.root_a, cross_mounts: opts.cross_mounts, side: PairSide::A,
        path: crate::vfs::sync_path(endpoints.a, endpoints.root_a, &rel_a)?, rel: rel_a,
        files: group.a.clone(), keep: None };
    let mut b = Side { backend: endpoints.b, root: endpoints.root_b, cross_mounts: opts.cross_mounts, side: PairSide::B,
        path: crate::vfs::sync_path(endpoints.b, endpoints.root_b, &rel_b)?, rel: rel_b,
        files: group.b.clone(), keep: None };
    progress(ResolvePhase::Preparing);
    for side in [&a, &b] {
        super::apply_boundary::guard(side.backend, side.root, &side.rel, opts.cross_mounts)?;
        side.verify(cancel)?;
    }
    progress(ResolvePhase::BackingUp);
    // Every exact variant is recoverable before any update or removal.
    for side in [&a, &b] {
        for variant in &side.files {
            super::apply_transaction::gate(cancel, Some(scope.sink))?;
            super::duplicate_backup::save_scoped(&side.version_side(), &side.path, &side.rel,
                scope.versions, variant, cancel)?;
        }
    }
    a.verify(cancel)?; b.verify(cancel)?;
    if let Some(selected) = selected.as_ref() {
        let (source, destination) = if keep_a { (&mut a, &mut b) } else { (&mut b, &mut a) };
        source.keep = Some(selected.clone());
        destination.keep = destination.files.iter().find(|variant| selected.same_content(variant)).cloned();
        if destination.keep.is_none() {
            progress(ResolvePhase::Copying);
            let outcome = publish(source, destination, selected, cancel, &throttle, scope)?;
            scope.sink.completed(CompletedAction { rel: conflict.rel.clone(),
                kind: CompletedKind::Copied { from: source.side },
                src_sig: Some(outcome.source), dst_sig: Some(outcome.destination), durable: true });
        }
    }
    progress(ResolvePhase::Deleting);
    remove_extras(&mut a, &b, &conflict.rel, cancel, scope)?;
    remove_extras(&mut b, &a, &conflict.rel, cancel, scope)?;
    progress(ResolvePhase::ReadingSignatures);
    let finish = AtomicBool::new(false);
    a.verify(&finish)?; b.verify(&finish)?;
    if a.files.len() > 1 || b.files.len() > 1 { return Err(changed()); }
    match selected {
        Some(ref selected) if !a.files.first().is_some_and(|variant| selected.same_content(variant))
            || !b.files.first().is_some_and(|variant| selected.same_content(variant)) => return Err(changed()),
        None if !a.files.is_empty() || !b.files.is_empty() => return Err(changed()),
        _ => {}
    }
    let result = (a.keep.as_ref().map(|variant| variant.signature), b.keep.as_ref().map(|variant| variant.signature));
    if result == (None, None) {
        scope.sink.completed(CompletedAction { rel: conflict.rel.clone(),
            kind: CompletedKind::Deleted { side: if keep_a { PairSide::B } else { PairSide::A } },
            src_sig: None, dst_sig: None, durable: true });
    } else {
        scope.sink.completed(CompletedAction { rel: conflict.rel.clone(),
            kind: CompletedKind::Copied { from: PairSide::A }, src_sig: result.0, dst_sig: result.1, durable: true });
    }
    Ok(result)
}

fn publish(source: &Side<'_>, destination: &mut Side<'_>, selected: &FileVariant,
    cancel: &AtomicBool, throttle: &Throttle, scope: &ApplyScope<'_>) -> io::Result<super::apply_stage::CopyOutcome> {
    super::apply_transaction::gate(cancel, Some(scope.sink))?;
    super::apply_boundary::target(destination.backend, destination.root, &destination.rel, Some(selected.content_size))?;
    let source_metadata = metadata_named(source.backend, &source.path, source.name())?
        .into_iter().find(|meta| meta.id == selected.id).ok_or_else(changed)?;
    let destination_metadata = destination.files.first().map(|variant| {
        metadata_named(destination.backend, &destination.path, destination.name())?
            .into_iter().find(|meta| meta.id == variant.id).ok_or_else(changed)
    }).transpose()?;
    let current = CapturedFile { metadata: destination_metadata };
    let source_state = CapturedFile { metadata: Some(source_metadata) };
    let staged = super::apply_stage::stage(source.backend, &source.path, &source_state,
        ExpectedFile::Present(selected.signature), destination.backend, &destination.path, &current,
        crate::vfs::StageDurability::Now, throttle, cancel, |_| {})?;
    if staged.bytes.bytes != selected.content_size || staged.bytes.hex() != selected.content_md5 { return Err(changed()); }
    source.verify(cancel)?; destination.verify(cancel)?;
    super::apply_transaction::gate(cancel, Some(scope.sink))?;
    let replaced = destination.files.first().cloned();
    let outcome = staged.publish(&destination.path, &current, true, cancel)?;
    super::apply_stage::require_durable(outcome.durable)?;
    let finish = AtomicBool::new(false);
    let after = observe_named(destination.backend, &destination.path, destination.name(), None, &finish)?;
    let keep = match &replaced {
        Some(file) => after.iter().find(|variant| variant.id == file.id),
        None => after.first().filter(|_| after.len() == 1),
    }.filter(|variant| selected.same_content(variant)).cloned().ok_or_else(changed)?;
    let old_others: Vec<_> = destination.files.iter().filter(|variant| Some(*variant) != replaced.as_ref()).collect();
    let new_others: Vec<_> = after.iter().filter(|variant| variant.id != keep.id).collect();
    if old_others != new_others { return Err(changed()); }
    destination.keep = Some(keep);
    destination.files = after;
    Ok(outcome)
}

fn remove_extras(side: &mut Side<'_>, other: &Side<'_>, rel: &str, cancel: &AtomicBool, scope: &ApplyScope<'_>) -> io::Result<()> {
    let extras: Vec<_> = side.files.iter().filter(|variant| Some(*variant) != side.keep.as_ref()).cloned().collect();
    for extra in extras {
        super::apply_transaction::gate(cancel, Some(scope.sink))?;
        side.verify(cancel)?; other.verify(cancel)?;
        super::apply_transaction::gate(cancel, Some(scope.sink))?;
        side.backend.remove_file_id(&side.path, extra.id.as_deref())?;
        super::apply_stage::require_durable(super::apply_stage::namespace(side.backend, &side.path)?)?;
        side.files.retain(|variant| variant != &extra);
        if let Some(keep) = &side.keep {
            scope.sink.completed(CompletedAction { rel: rel.to_string(),
                kind: CompletedKind::Copied { from: side.side.other() },
                src_sig: other.keep.as_ref().map(|variant| variant.signature), dst_sig: Some(keep.signature), durable: true });
        }
        let finish = AtomicBool::new(false);
        side.verify(&finish)?;
    }
    Ok(())
}
