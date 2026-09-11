//! Read-only upload collection and shared destination-name reservation.
use super::entries::{validate_transfer_name, TransferCollectionBudget};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub(super) struct UploadEntry {
    pub(super) src: PathBuf,
    pub(super) rel: String,
    pub(super) size: u64,
}

#[derive(Default)]
pub(super) struct UploadPlan {
    pub(super) files: Vec<UploadEntry>,
    pub(super) dirs: Vec<String>,
}

pub(super) struct DestinationNames {
    reserved: HashSet<String>,
    case_sensitive: bool,
}

impl DestinationNames {
    pub(super) fn new(backend: &dyn crate::vfs::Backend, root: &str) -> Self {
        Self { reserved: HashSet::new(), case_sensitive: backend.case_sensitive_paths(root) }
    }

    pub(super) fn reserve(
        &mut self, backend: &dyn crate::vfs::Backend, parent: &str, base: &str, cancel: &AtomicBool,
    ) -> Result<String, String> {
        for index in 1..=super::REMOTE_UNIQUE_ATTEMPTS {
            super::cancel::check(cancel)?;
            let name = super::numbered_remote_name(base, index);
            let path = super::rjoin(parent, &name);
            // Conservative reservation when a provider cannot promise distinct
            // case variants. The provider's non-replacing copy publication
            // remains responsible for aliases and concurrent creators; ID-based
            // providers do not promise atomic sibling-name reservations.
            let key = if self.case_sensitive { path.clone() } else { path.to_uppercase() };
            if self.reserved.contains(&key) { continue; }
            let exists = backend.try_exists(&path);
            super::cancel::check(cancel)?;
            match exists {
                Ok(false) => { self.reserved.insert(key); return Ok(name); }
                Ok(true) => {}
                Err(error) => return Err(format!("Ziel prüfen „{path}“: {error}")),
            }
        }
        Err(format!("Kein freier Name nach {} Versuchen", super::REMOTE_UNIQUE_ATTEMPTS))
    }
}

pub(super) fn collect_paths(
    backend: &dyn crate::vfs::Backend, paths: &[String], destination: &str, cancel: &AtomicBool,
) -> Result<UploadPlan, String> {
    super::cancel::check(cancel)?;
    let mut plan = UploadPlan::default();
    let mut budget = TransferCollectionBudget::default();
    let mut names = DestinationNames::new(backend, destination);
    for path in paths {
        super::cancel::check(cancel)?;
        let source = Path::new(path);
        let base = source.file_name().and_then(|name| name.to_str())
            .filter(|name| !name.is_empty()).ok_or_else(|| format!("{}: ungültiger Dateiname", source.display()))?;
        validate_transfer_name(base, path)?;
        budget.ensure_text_fits(&[path, base])?;
        let target = names.reserve(backend, destination, base, cancel)?;
        collect(source, target, &mut plan, &mut budget, 0, cancel)?;
    }
    Ok(plan)
}

fn collect(
    path: &Path, relative: String, plan: &mut UploadPlan,
    budget: &mut TransferCollectionBudget, depth: usize, cancel: &AtomicBool,
) -> Result<(), String> {
    super::cancel::check(cancel)?;
    let display = path.to_string_lossy();
    let name = path.file_name().and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: ungültiger Dateiname", path.display()))?;
    validate_transfer_name(name, &display)?;
    budget.record_node(depth, &[&display, &relative, name])?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    super::cancel::check(cancel)?;
    if super::super::upload_is_link_like(&metadata) {
        return Err(format!("{}: Links und Reparse-Punkte werden nicht hochgeladen", path.display()));
    }
    if metadata.is_dir() {
        plan.dirs.push(relative.clone());
        for entry in std::fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))? {
            super::cancel::check(cancel)?;
            let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
            let name = entry.file_name().into_string()
                .map_err(|_| format!("{}: Dateiname ist kein gültiges Unicode", entry.path().display()))?;
            validate_transfer_name(&name, &display)?;
            budget.ensure_text_fits(&[&name])?;
            collect(&entry.path(), format!("{relative}/{name}"), plan, budget, depth + 1, cancel)?;
        }
    } else if metadata.is_file() {
        plan.files.push(UploadEntry { src: path.to_path_buf(), rel: relative, size: metadata.len() });
    } else {
        return Err(format!("{}: Nur reguläre Dateien und Verzeichnisse werden hochgeladen", path.display()));
    }
    Ok(())
}
