//! Preserve empty directories only for an unfiltered, structured selection.
use super::{
    path_guard,
    relative::{rel_from_root, safe_rel_path},
};
use crate::types::{CopyOptions, FileEntry};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectoryPolicy {
    Preserve,
    MatchingFilesOnly,
}

pub(super) fn prepare(
    entries: &[FileEntry],
    root: &str,
    options: &CopyOptions,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !options.preserve_structure {
        return Ok(());
    }
    for entry in entries
        .iter()
        .filter(|entry| entry.is_dir && !entry.is_symlink)
    {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let relative = safe_rel_path(&rel_from_root(&entry.path, root)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ungueltiger relativer Ordnerpfad",
            )
        })?;
        path_guard::prepare_target_directory(&options.dest, &options.dest.join(relative))?;
    }
    Ok(())
}
