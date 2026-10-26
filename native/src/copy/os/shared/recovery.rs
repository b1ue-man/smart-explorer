//! Receipt-bound cleanup and restore of local transfer remnants.
use crate::local_access::ConfinedDirectory;
use std::{ffi::OsStr, fs::File, io, path::Path};

/// Restore an uncommitted move without replacement, or remove its own
/// quarantine only after the caller has confirmed the published target.
pub(crate) fn recover_transfer_quarantine(
    source: &Path,
    quarantine: &Path,
    expected_identity: &str,
    published: bool,
) -> io::Result<()> {
    let source_name = name(source)?;
    let quarantine_name = name(quarantine)?;
    let text = quarantine_name.to_str().ok_or_else(invalid)?;
    let (prefix, suffix) = text.rsplit_once(".smart-explorer-").ok_or_else(invalid)?;
    let nonce = suffix.strip_suffix(".move").ok_or_else(invalid)?;
    if prefix.is_empty()
        || !nonce_valid(nonce)
        || crate::vfs::fit_stage_name(
            ".",
            &source_name.to_string_lossy(),
            &format!(".smart-explorer-{nonce}.move"),
        ) != text
    {
        return Err(invalid());
    }
    let parent = ConfinedDirectory::open_root(parent_path(source))?;
    let quarantine_parent = ConfinedDirectory::open_root(parent_path(quarantine))?;
    if parent.identity()? != quarantine_parent.identity()? {
        return Err(invalid());
    }
    let result = super::super::platform::recover_transfer_child(
        &parent,
        quarantine_name,
        expected_identity,
        if published { None } else { Some(source_name) },
    );
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if published {
                return Ok(());
            }
            if super::publication::local_file_identity_handle(&parent.entry(source_name)?)?
                == expected_identity
            {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "restored transfer source identity changed",
                ))
            }
        }
        result => result,
    }
}

/// During a running transfer the retained source handle prevents object-ID
/// reuse even on a filesystem without a persistable incarnation capability.
/// Mutation still goes through private capture or the same Windows DELETE
/// handle; the absence of a receipt must never select path-only deletion.
pub(super) fn recover_transfer_quarantine_live(
    parent: &ConfinedDirectory,
    source: &Path,
    quarantine: &Path,
    expected: &File,
    published: bool,
) -> io::Result<()> {
    let source_name = name(source)?;
    let quarantine_name = name(quarantine)?;
    let text = quarantine_name.to_str().ok_or_else(invalid)?;
    let (prefix, suffix) = text.rsplit_once(".smart-explorer-").ok_or_else(invalid)?;
    let nonce = suffix.strip_suffix(".move").ok_or_else(invalid)?;
    if prefix.is_empty()
        || !nonce_valid(nonce)
        || crate::vfs::fit_stage_name(
            ".",
            &source_name.to_string_lossy(),
            &format!(".smart-explorer-{nonce}.move"),
        ) != text
    {
        return Err(invalid());
    }
    let current_parent = ConfinedDirectory::open_root(parent_path(source))?;
    let quarantine_parent = ConfinedDirectory::open_root(parent_path(quarantine))?;
    if parent.identity()? != current_parent.identity()?
        || parent.identity()? != quarantine_parent.identity()?
    {
        return Err(invalid());
    }
    match super::super::platform::recover_transfer_child_live(
        parent,
        quarantine_name,
        expected,
        if published { None } else { Some(source_name) },
    ) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if published {
                return Ok(());
            }
            if super::super::platform::same_file_object(&parent.entry(source_name)?, expected)? {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "restored live transfer source changed",
                ))
            }
        }
        result => result,
    }
}

/// Discard only an app-private copy stage named by a durable file receipt.
pub(crate) fn discard_transfer_stage(stage: &Path, expected_identity: &str) -> io::Result<()> {
    let name = name(stage)?;
    let text = name.to_str().ok_or_else(invalid)?;
    let parts = crate::vfs::split_stage_name(text).ok_or_else(invalid)?;
    if text.ends_with(".move") || parts.base.is_none() {
        return Err(invalid());
    }
    let parent = ConfinedDirectory::open_root(parent_path(stage))?;
    match super::super::platform::recover_transfer_child(&parent, name, expected_identity, None) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(super) fn discard_transfer_stage_live(
    parent: &ConfinedDirectory,
    stage: &Path,
    expected: &File,
) -> io::Result<()> {
    let name = name(stage)?;
    let text = name.to_str().ok_or_else(invalid)?;
    let parts = crate::vfs::split_stage_name(text).ok_or_else(invalid)?;
    if text.ends_with(".move") || parts.base.is_none() {
        return Err(invalid());
    }
    let current = ConfinedDirectory::open_root(parent_path(stage))?;
    if parent.identity()? != current.identity()? {
        return Err(invalid());
    }
    match super::super::platform::recover_transfer_child_live(parent, name, expected, None) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn name(path: &Path) -> io::Result<&OsStr> {
    path.file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(invalid)
}
fn parent_path(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
fn nonce_valid(nonce: &str) -> bool {
    nonce.len() == 16
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "transfer recovery requires its exact private sibling and retained parent",
    )
}
