//! Fresh sync observations protect omitted siblings only at their own name.
//! Enumeration failures still fail the observation; incomplete listings never
//! establish absence or allow an ID-addressed mutation.
use crate::vfs::{self, Backend, VfsListing, VfsMeta};
use std::io;

pub(super) fn named(
    backend: &dyn Backend,
    parent: &str,
    literal_name: &str,
) -> io::Result<Vec<VfsMeta>> {
    vfs::sync_child_path(backend, parent, literal_name)?;
    matching(backend, parent, |name, _| name == literal_name)
}

/// Resolve a file locator through its provider's child contract rather than
/// treating an encoded final path segment as the file's literal name.
pub(super) fn at_path(backend: &dyn Backend, path: &str) -> io::Result<Vec<VfsMeta>> {
    let parent = super::paths::parent_of(path)
        .ok_or_else(|| super::apply_guard::drift("sync file has no parent"))?;
    matching(backend, &parent, |_, child| child == path)
}

fn matching(
    backend: &dyn Backend,
    parent: &str,
    matches: impl Fn(&str, &str) -> bool,
) -> io::Result<Vec<VfsMeta>> {
    let Some(listing) = directory(backend, parent)? else {
        return Ok(Vec::new());
    };
    for omitted in listing.omitted {
        // An unaddressable omission cannot prove which child was protected.
        let child = vfs::sync_child_path(backend, parent, &omitted.rel)
            .map_err(|_| super::apply_boundary::protected(super::OmissionKind::NotRepresentable))?;
        if matches(&omitted.rel, &child) {
            return Err(super::apply_boundary::protected(omitted.reason.into()));
        }
    }
    let mut entries = Vec::new();
    for entry in listing.entries {
        // Sync uses the provider's literal child contract. The recursive
        // deletion validator has stricter native-path rules (e.g. backslash)
        // and must not reject a valid Drive sibling or selected literal file.
        let child = vfs::sync_child_path(backend, parent, &entry.name)?;
        if matches(&entry.name, &child) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn directory(backend: &dyn Backend, path: &str) -> io::Result<Option<VfsListing>> {
    backend.invalidate_cache();
    let metadata = match vfs::sync_stat(backend, path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.is_symlink || !metadata.is_dir || metadata.special {
        return Err(super::apply_boundary::protected(if metadata.is_symlink {
            super::OmissionKind::Link
        } else {
            super::OmissionKind::Special
        }));
    }
    // A parent that vanishes after its stat leaves an incomplete enumeration;
    // only the initial exact parent observation can establish absence.
    vfs::list_dir_tolerant(backend, path).map(Some)
}
