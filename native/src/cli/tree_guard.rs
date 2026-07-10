use crate::vfs::{Backend, VfsMeta};
use std::collections::HashSet;
use std::io;

#[derive(Clone)]
pub(super) enum DestinationState {
    Missing,
    Directory(VfsMeta),
    File(VfsMeta),
}

pub(super) fn inspect_destination(
    backend: &dyn Backend,
    path: &str,
    source_is_dir: bool,
    force: bool,
) -> Result<DestinationState, String> {
    match backend.stat(path) {
        Ok(metadata) if metadata.is_symlink && source_is_dir => Err(format!(
            "destination exists and is not a plain directory (link-like and unsafe): {path}"
        )),
        Ok(metadata) if metadata.is_symlink => Err(format!(
            "destination exists and is not a regular file (link-like and unsafe): {path}"
        )),
        Ok(metadata) if source_is_dir && metadata.is_dir => {
            Ok(DestinationState::Directory(metadata))
        }
        Ok(_) if source_is_dir => Err(format!(
            "destination exists and is not a plain directory: {path}"
        )),
        Ok(metadata) if metadata.is_dir => Err(format!(
            "destination exists and is not a regular file: {path}"
        )),
        Ok(_) if !force => Err(format!("destination exists; pass --force: {path}")),
        Ok(metadata) => Ok(DestinationState::File(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DestinationState::Missing),
        Err(error) => Err(format!("cannot inspect destination {path}: {error}")),
    }
}

pub(super) fn validate_destination_state(
    backend: &dyn Backend,
    path: &str,
    expected: &DestinationState,
    must_be_directory: bool,
) -> Result<(), String> {
    match (expected, backend.stat(path)) {
        (DestinationState::Missing, Err(error)) if error.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        (DestinationState::Missing, Ok(_)) => {
            Err(format!("destination appeared after preflight: {path}"))
        }
        (DestinationState::Missing, Err(error)) => Err(format!(
            "destination changed after preflight: {path}: {error}"
        )),
        (DestinationState::Directory(before), Ok(after)) if must_be_directory => {
            validate_same_destination(before, &after, path, true)
        }
        (DestinationState::File(before), Ok(after)) if !must_be_directory => {
            validate_same_destination(before, &after, path, false)
        }
        (_, Ok(_)) => Err(format!("destination changed type after preflight: {path}")),
        (_, Err(error)) => Err(format!(
            "destination changed after preflight: {path}: {error}"
        )),
    }
}

pub(super) fn validate_same_source(
    expected: &VfsMeta,
    actual: &VfsMeta,
    path: &str,
) -> Result<(), String> {
    let identity_changed = expected.id != actual.id;
    let time_changed = !expected.is_dir
        && expected.mtime_ms != 0
        && actual.mtime_ms != 0
        && expected.mtime_ms != actual.mtime_ms;
    let hash_changed = !expected.is_dir && expected.content_md5 != actual.content_md5;
    if expected.name != actual.name
        || expected.is_dir != actual.is_dir
        || expected.is_symlink != actual.is_symlink
        || actual.is_symlink
        || (!expected.is_dir && expected.size != actual.size)
        || identity_changed
        || time_changed
        || hash_changed
    {
        Err(format!("source type, identity, or size changed: {path}"))
    } else {
        Ok(())
    }
}

pub(super) fn validate_listing(
    listed: &VfsMeta,
    fresh: &VfsMeta,
    path: &str,
) -> Result<(), String> {
    if listed.name != fresh.name
        || listed.is_dir != fresh.is_dir
        || listed.is_symlink != fresh.is_symlink
        || (!listed.is_dir && listed.size != fresh.size)
        || listed
            .id
            .as_deref()
            .is_some_and(|id| fresh.id.as_deref() != Some(id))
    {
        Err(format!(
            "source changed while it was being preflighted: {path}"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn validate_listed_names(path: &str, listed: &[VfsMeta]) -> Result<(), String> {
    let mut names = HashSet::with_capacity(listed.len());
    for child in listed {
        crate::vfs::validate_child_name(&child.name).map_err(|error| error.to_string())?;
        if !names.insert(child.name.as_str()) {
            return Err(format!(
                "backend returned duplicate child name in {path}: {:?}",
                child.name
            ));
        }
    }
    Ok(())
}

fn validate_same_destination(
    expected: &VfsMeta,
    actual: &VfsMeta,
    path: &str,
    directory: bool,
) -> Result<(), String> {
    let identity_changed = expected.id != actual.id;
    if actual.is_symlink
        || actual.is_dir != directory
        || identity_changed
        || (!directory
            && (expected.size != actual.size
                || (expected.mtime_ms != 0
                    && actual.mtime_ms != 0
                    && expected.mtime_ms != actual.mtime_ms)
                || expected.content_md5 != actual.content_md5))
    {
        Err(format!("destination changed after preflight: {path}"))
    } else {
        Ok(())
    }
}

pub(super) fn reject_link(metadata: &VfsMeta, path: &str) -> Result<(), String> {
    if metadata.is_symlink {
        Err(format!("link-like source is not copied: {path}"))
    } else {
        Ok(())
    }
}

pub(super) fn metadata_text_bytes(metadata: &VfsMeta) -> u64 {
    (metadata.name.len() as u64)
        .saturating_add(metadata.id.as_ref().map_or(0, |id| id.len() as u64))
        .saturating_add(
            metadata
                .content_md5
                .as_ref()
                .map_or(0, |hash| hash.len() as u64),
        )
}

pub(super) fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), child)
    }
}
