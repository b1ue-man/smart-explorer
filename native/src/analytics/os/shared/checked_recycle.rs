//! Verify and capture a regular child before the OS publishes a reversible trash record.
use crate::{
    local_access::{DirectoryHandle, QuarantinedChild},
    vfs::{RecycleExpectation, RecycleOutcome},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    path::{Component, Path},
};

pub(crate) fn recycle(
    root: &Path,
    path: &Path,
    expected: &RecycleExpectation,
    publish: impl FnOnce(&mut QuarantinedChild, &Path) -> io::Result<()>,
) -> io::Result<RecycleOutcome> {
    with_regular_child(root, path, |_, path, directory, name, file| {
        if !matches(file, expected)? {
            return Ok(RecycleOutcome::Changed);
        }
        let mut captured = directory.quarantine_regular_child(name, file)?;
        let result = (|| {
            if !matches(captured.file(), expected)? {
                return Ok(RecycleOutcome::Changed);
            }
            publish(&mut captured, path)?;
            Ok(RecycleOutcome::Recycled)
        })();
        if !matches!(result, Ok(RecycleOutcome::Recycled)) {
            if let Err(restore) = captured.restore() {
                let cause = result
                    .err()
                    .map_or("Datei wurde verändert".into(), |error| error.to_string());
                return Err(io::Error::new(
                    restore.kind(),
                    format!(
                        "{cause}; Wiederherstellen fehlgeschlagen: {restore}; Inhalt bleibt in {}",
                        captured.retained_location().display()
                    ),
                ));
            }
        }
        result
    })
}

/// Select the ordinary confined object; callers may persist an intent before
/// capture without reopening the final child or granting elevated reads.
pub(super) fn with_regular_child<T>(
    root: &Path,
    path: &Path,
    selected: impl FnOnce(&Path, &Path, &DirectoryHandle, &OsStr, &File) -> io::Result<T>,
) -> io::Result<T> {
    let root = std::fs::canonicalize(root)?;
    let path = crate::local_access::normalize_scan_root(path);
    let relative = path.strip_prefix(&root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Papierkorbpfad liegt außerhalb der autorisierten Wurzel",
        )
    })?;
    let mut directory = DirectoryHandle::open_root(&root)?;
    let mut parts = relative.components().peekable();
    let name = loop {
        let Some(Component::Normal(name)) = parts.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Papierkorb braucht einen regulären Child-Pfad",
            ));
        };
        if parts.peek().is_none() {
            break name;
        }
        directory = directory.open_child(name)?;
    };
    let file = directory.open_regular_child(name)?;
    selected(&root, &path, &directory, name, &file)
}
fn matches(file: &File, expected: &RecycleExpectation) -> io::Result<bool> {
    let before = file.metadata()?;
    if !before.is_file() || before.len() != expected.size {
        return Ok(false);
    }
    if let Some(expected_hash) = &expected.sha256 {
        let mut read = file.try_clone()?;
        // Both verifications start at byte zero even when clone shares a cursor.
        use std::io::{Seek, SeekFrom};
        read.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        let mut bytes = 0u64;
        loop {
            let n = match read.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                break;
            }
            bytes = bytes.saturating_add(n as u64);
            if bytes > expected.size {
                return Ok(false);
            }
            digest.update(&buffer[..n]);
        }
        if bytes != expected.size || format!("{:x}", digest.finalize()) != *expected_hash {
            return Ok(false);
        }
    }
    let after = file.metadata()?;
    Ok(before.len() == after.len() && before.modified().ok() == after.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_recycle_changed_content_is_not_captured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy");
        std::fs::write(&path, b"changed").unwrap();
        let expected = RecycleExpectation {
            size: 4,
            sha256: Some(format!("{:x}", Sha256::digest(b"same"))),
        };
        let result = recycle(dir.path(), &path, &expected, |_, _| {
            panic!("Changed content must not reach trash")
        })
        .unwrap();
        assert_eq!(result, RecycleOutcome::Changed);
        assert_eq!(std::fs::read(path).unwrap(), b"changed");
    }
    #[test]
    fn review_task_recycle_publication_failure_restores_without_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy");
        std::fs::write(&path, b"same").unwrap();
        let expected = RecycleExpectation {
            size: 4,
            sha256: Some(format!("{:x}", Sha256::digest(b"same"))),
        };
        let result = recycle(dir.path(), &path, &expected, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trash unavailable",
            ))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(path).unwrap(), b"same");
    }
}
