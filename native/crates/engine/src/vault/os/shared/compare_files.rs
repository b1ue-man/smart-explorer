//! Small file operations on the encrypted storage of a vault, shared by the
//! actions that change cipher files themselves („Beide behalten“, folder
//! actions, structure fixes; vault plan C6.4, U26-6, U26-7): publishing a
//! copy under a new name without replacing anything, replacing a small file
//! through a stage, removing a tree without following links, and the copy of
//! cipher files to `vault-removed/` that comes before every removal. Only
//! ciphertext and storage names pass here; no key.
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::vault::core::path_map::{child, storage_path};
use crate::vfs::{self, Backend};

/// `<app data>/vault-removed`: the cipher files of removed entries; never
/// emptied by se (U26-7).
pub(super) fn removed_root() -> PathBuf {
    crate::support_dirs::app_data_dir().join("vault-removed")
}

/// `vault-removed/<site>/<JJJJMMTT-HHMMSS>`, the folder one removal copies to.
pub(super) fn backup_dir(site: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    removed_root().join(site).join(stamp)
}

/// Removes a stage this process created and never published.
pub(super) fn discard_stage(inner: &dyn Backend, stage: &str) {
    match inner.discard_copy_stage(stage) {
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            let _ = inner.remove_file(stage);
        }
        _ => {}
    }
}

/// Copies the file `source` byte for byte to `dest`, which must not exist:
/// through a stage of the storage and its no-replace publication.
pub(super) fn copy_new(inner: &dyn Backend, source: &str, dest: &str) -> io::Result<()> {
    let size = inner.stat(source)?.size;
    let stage = vfs::unique_staging_path(inner, dest, "compare")?;
    let mut reader = inner.open_read(source)?;
    let mut writer = inner.open_write_copy_stage_sized(&stage, size)?;
    let copied = io::copy(&mut reader, &mut writer).and_then(|_| writer.flush());
    drop(writer);
    match copied {
        Ok(()) => inner.promote_copy_stage(&stage, dest).inspect_err(|_| {
            discard_stage(inner, &stage);
        }),
        Err(error) => {
            discard_stage(inner, &stage);
            Err(error)
        }
    }
}

/// Writes a new small file exclusively (`AlreadyExists` when taken).
pub(super) fn put_new(inner: &dyn Backend, path: &str, bytes: &[u8]) -> io::Result<()> {
    super::password::write_new(inner, path, bytes)
}

/// Replaces the file `dest` (or creates it) with `bytes` through a stage.
pub(super) fn replace_bytes(inner: &dyn Backend, dest: &str, bytes: &[u8]) -> io::Result<()> {
    let stage = vfs::unique_staging_path(inner, dest, "compare")?;
    let mut writer = inner.open_write_copy_stage_sized(&stage, bytes.len() as u64)?;
    let written = writer.write_all(bytes).and_then(|()| writer.flush());
    drop(writer);
    match written {
        Ok(()) => inner.promote_staged(&stage, dest).inspect_err(|_| {
            discard_stage(inner, &stage);
        }),
        Err(error) => {
            discard_stage(inner, &stage);
            Err(error)
        }
    }
}

/// Removes the file or folder tree at `path`; a link is removed itself and
/// never followed.
pub(super) fn remove_tree(inner: &dyn Backend, path: &str) -> io::Result<()> {
    let meta = inner.stat(path)?;
    if meta.is_dir && !meta.is_symlink {
        for entry in inner.list_dir(path)? {
            remove_tree(inner, &child(path.trim_end_matches('/'), &entry.name))?;
        }
        inner.remove_dir(path)
    } else {
        inner.remove_file(path)
    }
}

/// Copies the cipher file or folder tree `rel` of the storage `root` into
/// `into/<rel>` on this device (U26-7: before any removal; a failure stops
/// the removal). A file that is already there from the same second stays.
pub(super) fn backup_tree(
    inner: &dyn Backend,
    root: &str,
    rel: &str,
    into: &Path,
) -> io::Result<()> {
    let path = storage_path(root, rel);
    let meta = inner.stat(&path)?;
    let target = into.join(rel);
    if meta.is_dir && !meta.is_symlink {
        std::fs::create_dir_all(&target)?;
        for entry in inner.list_dir(&path)? {
            backup_tree(inner, root, &child(rel, &entry.name), into)?;
        }
        return Ok(());
    }
    if meta.is_symlink || meta.special {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut reader = inner.open_read(&path)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
    {
        Ok(mut file) => {
            io::copy(&mut reader, &mut file)?;
            file.sync_all()
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}
