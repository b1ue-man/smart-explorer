//! The runs of named folders of a keyed host walk (`hash_walk_keys_v1`,
//! `FsHashWalk.folders`): each folder below the request path read as the
//! whole walk reads it, without its subfolders, after its `Run` mark. Local
//! exports are read on the host's threads, connections one after the other.
//! A folder that cannot be resolved or opened is its own failure run, which
//! the client sees as a key other than the one of the first phase.
use std::collections::HashSet;
use std::io;

use super::{child_rel, Batch};
use crate::share::{
    fs_access::FsAccess,
    fs_response::{FsHashEntry, FsHashMeta, FsOmissionReason},
    wire::FsHashWalk,
};
use crate::vfs::Scheme;

/// The whole walk, or with `folders` only the named folders.
pub(super) fn visit(
    batch: &mut Batch<'_, '_>,
    request: &FsHashWalk,
    access: &FsAccess,
) -> io::Result<()> {
    if request.folders.is_empty() {
        return super::visit(batch, &request.path, access, "");
    }
    let mut seen = HashSet::new();
    let mut local = Vec::new();
    for rel in &request.folders {
        if !seen.insert(rel.as_str()) {
            continue;
        }
        batch.shared().check()?;
        let path = if rel.is_empty() {
            request.path.clone()
        } else {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            format!("{}/{rel}", request.path.trim_end_matches('/'))
        };
        if let Some(job) = one(batch, &path, access, rel)? {
            local.push(job);
        }
    }
    batch.send()?;
    super::local::walk_folders(batch.shared(), local)
}

/// One folder: its run now, or the local folder to read on the threads.
fn one(
    batch: &mut Batch<'_, '_>,
    path: &str,
    access: &FsAccess,
    rel: &str,
) -> io::Result<Option<super::local::Job>> {
    let depth = rel.split('/').filter(|part| !part.is_empty()).count();
    let parts = crate::share::fs::split_clean(path)?;
    if access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        batch.begin_folder(rel)?;
        for entry in access.list_dir(path)? {
            crate::vfs::validate_child_name(&entry.name)?;
            let meta = batch.shared().meta(FsHashMeta::default);
            batch.add(Ok(FsHashEntry {
                rel: child_rel(rel, &entry.name),
                is_dir: true,
                meta,
                ..FsHashEntry::default()
            }))?;
        }
        batch.end_folder()?;
        return Ok(None);
    }
    batch.begin_folder(rel)?;
    let target = match access.resolve(path) {
        Ok(target) => target,
        Err(error) => {
            batch.failed(rel, error)?;
            batch.end_folder()?;
            return Ok(None);
        }
    };
    if target.backend.scheme() != Scheme::Local {
        super::remote::folder(batch, &*target.backend, &target.path, rel, depth)?;
        batch.end_folder()?;
        return Ok(None);
    }
    let physical = crate::share::storage_roots::local_path(&target)?;
    if crate::share::storage_roots::is_private(&physical) {
        batch.hole(
            rel,
            FsOmissionReason::Unreadable,
            "Interner Host-Speicher ist geschützt",
        )?;
        batch.end_folder()?;
        return Ok(None);
    }
    match crate::share::storage_roots::open_local(&target) {
        Ok(handle) => {
            // Its run is read on the threads (`local::walk_folders`).
            batch.abandon_folder();
            Ok(Some((handle, physical, rel.to_string(), depth)))
        }
        Err(error) => {
            batch.failed(rel, error)?;
            batch.end_folder()?;
            Ok(None)
        }
    }
}
