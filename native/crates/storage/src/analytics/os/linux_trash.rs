//! Freedesktop Trash publication from a captured file; every move uses opened directories.
use crate::local_access::{DirectoryHandle, QuarantinedChild};
use ring::rand::{SecureRandom, SystemRandom};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::{
    ffi::OsStr,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(super) fn publish(captured: &mut QuarantinedChild, original: &Path) -> io::Result<()> {
    let source = captured.file().metadata()?;
    let uid = unsafe { libc::geteuid() };
    let data = crate::support_dirs::app_data_dir();
    let data = data
        .parent()
        .ok_or_else(|| io::Error::other("XDG-Datenwurzel fehlt"))?;
    let home = DirectoryHandle::open_root(data)?;
    let (trash, origin) = if home.metadata()?.dev() == source.dev() {
        (
            private(&home, OsStr::new("Trash"), uid)?,
            original.as_os_str().as_bytes().to_vec(),
        )
    } else {
        let top = mount_root(original, source.dev())?;
        let parent = DirectoryHandle::open_root(&top)?;
        let trash = private(&parent, OsStr::new(&format!(".Trash-{uid}")), uid)?;
        let relative = original.strip_prefix(&top).map_err(io::Error::other)?;
        (trash, relative.as_os_str().as_bytes().to_vec())
    };
    if trash.metadata()?.dev() != source.dev() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Papierkorb liegt auf anderem Dateisystem",
        ));
    }
    let files = private(&trash, OsStr::new("files"), uid)?;
    let info = private(&trash, OsStr::new("info"), uid)?;
    let mut random = [0; 16];
    SystemRandom::new()
        .fill(&mut random)
        .map_err(|_| io::Error::other("Papierkorb-ID konnte nicht erzeugt werden"))?;
    let id = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // Claim the journal before moving data. An abandoned reservation contains
    // no payload and never grants a destructive fallback.
    let mut record = info.create_file_new(OsStr::new(&format!("{id}.trashinfo")))?;
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    write!(
        record,
        "[Trash Info]\nPath={}\nDeletionDate={date}\n",
        escape(&origin)
    )?;
    record.sync_all()?;
    captured.move_to(&files, OsStr::new(&id))?;
    Ok(())
}
fn private(parent: &DirectoryHandle, name: &OsStr, uid: u32) -> io::Result<DirectoryHandle> {
    let directory = match parent.create_private_child(name) {
        Ok(directory) => directory,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => parent.open_child(name)?,
        Err(e) => return Err(e),
    };
    let metadata = directory.metadata()?;
    if metadata.uid() != uid || metadata.mode() & 0o077 != 0 || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Papierkorb muss dem Benutzer gehören und privat sein",
        ));
    }
    Ok(directory)
}
fn mount_root(original: &Path, device: u64) -> io::Result<PathBuf> {
    let mut top = original
        .parent()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Papierkorbquelle hat keinen Parent",
            )
        })?
        .to_path_buf();
    while let Some(parent) = top.parent() {
        let metadata = std::fs::symlink_metadata(parent)?;
        if metadata.dev() != device || metadata.file_type().is_symlink() {
            break;
        }
        top = parent.to_path_buf();
    }
    Ok(top)
}
fn escape(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            output.push(*byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_freedesktop_record_escapes_literal_original_bytes() {
        assert_eq!(escape(b"/x/a% b\nc"), "/x/a%25%20b%0Ac");
    }
}
