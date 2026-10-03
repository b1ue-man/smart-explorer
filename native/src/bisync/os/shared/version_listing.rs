//! Bounded target/appdata version discovery, including compatible legacy runs.
use std::io;
use std::sync::atomic::AtomicBool;
use crate::vfs::{Backend, LocalBackend};
use super::paths::{join, validate_child_name};
use super::transfer_stream::check;
use super::version_manifest::{self as record, Manifest};
use super::versions::{VersionEntry, VersionSide, VersionStore};

pub(super) struct Managed {
    pub(super) entry: VersionEntry,
    pub(super) manifest: Manifest,
    pub(super) dir: String,
}

pub(super) fn children(backend: &dyn Backend, root: &str, cancel: &AtomicBool) -> io::Result<Vec<crate::vfs::VfsMeta>> {
    check(cancel)?;
    let meta = match backend.stat(root) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !meta.is_dir || meta.is_symlink || meta.special { return Err(record::invalid("versions directory is not plain")); }
    let listing = crate::vfs::list_dir_tolerant(backend, root)?;
    if !listing.omitted.is_empty() { return Err(record::invalid("versions listing is incomplete")); }
    if listing.entries.len() as u64 > super::SyncLimits::for_memory(crate::transfer::physical_memory()).walk_entries {
        return Err(record::invalid("versions directory exceeds its listing budget"));
    }
    for child in &listing.entries { validate_child_name(&child.name)?; }
    Ok(listing.entries)
}

pub(super) fn managed(backend: &dyn Backend, root: &str, pair: &str,
    store: VersionStore, cancel: &AtomicBool) -> io::Result<Vec<Managed>> {
    let limit = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut visited = 0usize;
    let mut bytes = 0usize;
    let mut result = Vec::new();
    for run in children(backend, root, cancel)? {
        budget(&mut visited,&mut bytes,&run.name,limit)?;
        if !run.is_dir || run.is_symlink || run.special || run.name.parse::<u64>().is_ok() { continue; }
        let run_path = join(root, &run.name);
        for side in children(backend, &run_path, cancel)? {
            budget(&mut visited,&mut bytes,&side.name,limit)?;
            if !side.is_dir || side.is_symlink || side.special { continue; }
            let side_path = join(&run_path, &side.name);
            for item in children(backend, &side_path, cancel)? {
                budget(&mut visited,&mut bytes,&item.name,limit)?;
                if !item.is_dir || item.is_symlink || item.special { continue; }
                let dir = join(&side_path, &item.name);
                let ready = join(&dir, "entry.json");
                let intent = join(&dir, "intent.json");
                let path = match backend.stat(&ready) {
                    Ok(_) => ready,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => intent,
                    Err(error) => return Err(error),
                };
                let manifest = match record::read(backend, &path, cancel) {
                    Ok(manifest) => manifest,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                if manifest.pair != pair { continue; }
                let meta = match backend.stat(&manifest.data) {
                    Ok(meta) => meta,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                if meta.is_dir || meta.is_symlink || meta.special {
                    return Err(record::invalid("version data is not a regular file"));
                }
                if meta.size != manifest.size { return Err(record::invalid("version data has changed")); }
                bytes = bytes.saturating_add(manifest.rel.len()).saturating_add(manifest.root.len());
                if bytes as u64 > limit.walk_text_bytes { return Err(record::invalid("version metadata exceeds its budget")); }
                let entry = manifest.entry(store)?;
                result.push(Managed { entry, manifest, dir });
            }
        }
    }
    Ok(result)
}

pub(super) fn list(pair: &str, sides: &[VersionSide<'_>], cancel: &AtomicBool) -> io::Result<Vec<VersionEntry>> {
    record::validate_pair(pair)?;
    let mut entries = Vec::new();
    for side in sides {
        entries.extend(managed(side.backend, &join(side.root, ".se-versions"), pair, VersionStore::SyncRoot, cancel)?
            .into_iter().filter(|item| item.manifest.belongs(pair, side)).map(|item| item.entry));
    }
    let app = super::persistence::versions_dir(pair);
    let root = app.to_str().ok_or_else(|| record::invalid("versions path is not Unicode"))?;
    let backend = LocalBackend::new(root);
    entries.extend(managed(&backend, root, pair, VersionStore::AppData, cancel)?.into_iter().map(|item| item.entry));
    entries.extend(legacy(&backend, root, cancel)?);
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.preserved_ms));
    Ok(entries)
}

pub(super) fn legacy(backend: &dyn Backend, root: &str, cancel: &AtomicBool) -> io::Result<Vec<VersionEntry>> {
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut result = Vec::new();
    let mut count = 0u64;
    let mut text = 0u64;
    let mut pending = Vec::new();
    for run in children(backend, root, cancel)? {
        if let Ok(stamp) = run.name.parse::<u64>() {
            if run.is_dir && !run.is_symlink && !run.special {
                pending.push((join(root, &run.name), String::new(), run.name, stamp));
            }
        }
    }
    while let Some((dir, rel, run, stamp)) = pending.pop() {
        for meta in children(backend, &dir, cancel)? {
            if meta.is_symlink || meta.special { return Err(record::invalid("legacy version has a protected entry")); }
            let child_rel = if rel.is_empty() { meta.name.clone() } else { format!("{rel}/{}", meta.name) };
            count = count.saturating_add(1);
            text = text.saturating_add(child_rel.len() as u64);
            if count > limits.walk_entries || text > limits.walk_text_bytes { return Err(record::invalid("legacy versions exceed their budget")); }
            let path = join(&dir, &meta.name);
            if meta.is_dir { pending.push((path, child_rel, run.clone(), stamp)); }
            else { result.push(VersionEntry { side: None, rel: child_rel, run_id: run.clone(),
                preserved_ms: i64::try_from(stamp.saturating_mul(1000)).unwrap_or(i64::MAX),
                reason: None, size: meta.size, mtime_ms: meta.mtime_ms, store: VersionStore::AppData,
                stored_path: path, job_id: None }); }
        }
    }
    Ok(result)
}


fn budget(count: &mut usize, text: &mut usize, name: &str, limits: super::SyncLimits) -> io::Result<()> {
    *count = count.saturating_add(1); *text = text.saturating_add(name.len());
    if *count as u64 > limits.walk_entries || *text as u64 > limits.walk_text_bytes {
        return Err(record::invalid("versions listing exceeds its collection budget"));
    }
    Ok(())
}
