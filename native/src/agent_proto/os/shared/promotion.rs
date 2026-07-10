use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use super::transfer::create_staged_file;

/// A private, flat spool directory. It is outside the destination namespace so
/// an invalid or interrupted tree cannot create destination children early.
pub(crate) struct StagingArea {
    directory: PathBuf,
}

impl StagingArea {
    pub(crate) fn create(purpose: &str, request_id: u64) -> io::Result<Self> {
        let base = std::fs::canonicalize(std::env::temp_dir())?;
        validate_destination_root(&base)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..1000u32 {
            let directory = base.join(format!(
                ".se-agent-{purpose}-{request_id:x}-{nonce:x}-{attempt:x}.spool"
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => {
                    if let Err(error) = super::local_platform::secure_staging_directory(&directory)
                    {
                        let _ = std::fs::remove_dir(&directory);
                        return Err(error);
                    }
                    return Ok(Self { directory });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate tree staging directory",
        ))
    }

    pub(crate) fn create_file(&self, index: u64, expected: u64) -> io::Result<StagedLocalFile> {
        let staged = self.directory.join(format!("{index:016x}.part"));
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        super::local_platform::secure_staging_file(&writer)?;
        Ok(StagedLocalFile {
            staged,
            writer: Some(writer),
            expected,
            received: 0,
            finished: false,
        })
    }
}

impl Drop for StagingArea {
    fn drop(&mut self) {
        // The spool is deliberately flat. Never recursively follow anything an
        // attacker might have inserted into a shared temporary directory.
        if let Ok(entries) = std::fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}

/// One complete incoming file held outside the destination namespace.
pub(crate) struct StagedLocalFile {
    staged: PathBuf,
    writer: Option<std::fs::File>,
    expected: u64,
    received: u64,
    finished: bool,
}

impl StagedLocalFile {
    pub(crate) fn write_chunk(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() > super::CHUNK {
            return Err(invalid("tree data frame exceeds the protocol chunk limit"));
        }
        let received = self
            .received
            .checked_add(data.len() as u64)
            .ok_or_else(|| invalid("file size overflow"))?;
        if received > self.expected {
            return Err(invalid("staged file exceeded its declared size"));
        }
        self.writer
            .as_mut()
            .ok_or_else(|| io::Error::other("staged writer is closed"))?
            .write_all(data)?;
        self.received = received;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        if self.received != self.expected {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "staged file ended at {} of {} bytes",
                    self.received, self.expected
                ),
            ));
        }
        let writer = self
            .writer
            .take()
            .ok_or_else(|| io::Error::other("staged writer is closed"))?;
        writer.sync_all()?;
        drop(writer);
        self.finished = true;
        Ok(())
    }

    pub(crate) fn copy_to_writer(&self, writer: &mut dyn Write) -> io::Result<u64> {
        if !self.finished {
            return Err(io::Error::other("tree spool file is incomplete"));
        }
        let metadata = std::fs::symlink_metadata(&self.staged)?;
        if !metadata.is_file() || super::local_platform::metadata_is_link_like(&metadata) {
            return Err(invalid("tree spool entry is not a regular file"));
        }
        if metadata.len() != self.expected {
            return Err(invalid("tree spool file changed size before publication"));
        }
        let mut reader = std::fs::File::open(&self.staged)?;
        let copied = io::copy(&mut reader, writer)?;
        if copied != self.expected {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "tree spool file changed while being published",
            ));
        }
        Ok(copied)
    }

    pub(crate) fn publish_local(
        self,
        destination: &Path,
        purpose: &str,
        request_id: u64,
    ) -> io::Result<()> {
        ensure_destination_parent_plain(destination)?;
        let destination_text = destination
            .to_str()
            .ok_or_else(|| invalid("tree destination path is not valid UTF-8"))?;
        let (publish_stage, mut writer) =
            create_staged_file(destination_text, purpose, request_id)?;
        let transfer = (|| {
            self.copy_to_writer(&mut writer)?;
            writer.sync_all()
        })();
        drop(writer);
        let result = transfer.and_then(|()| promote_staged_replace(&publish_stage, destination));
        if result.is_err() {
            let _ = std::fs::remove_file(&publish_stage);
        }
        result
    }
}

impl Drop for StagedLocalFile {
    fn drop(&mut self) {
        drop(self.writer.take());
        let _ = std::fs::remove_file(&self.staged);
    }
}

/// Read-only destination preflight. Missing components are allowed, but every
/// existing component (including the selected root) must be a plain directory.
pub(crate) fn validate_destination_root(path: &Path) -> io::Result<()> {
    let absolute = absolute_path(path)?;
    let mut current = PathBuf::new();
    let mut missing = false;
    for component in absolute.components() {
        push_component(&mut current, component)?;
        if !matches!(component, Component::Normal(_)) || missing {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => validate_plain_directory(&current, &metadata)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing = true,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn validate_file_destination(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("tree destination has no parent directory"))?;
    validate_destination_root(parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file() && !super::local_platform::metadata_is_link_like(&metadata) =>
        {
            Ok(())
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tree file destination is a directory or link-like entry",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn ensure_plain_directory_tree(path: &Path) -> io::Result<()> {
    let absolute = absolute_path(path)?;
    let mut current = PathBuf::new();
    for component in absolute.components() {
        push_component(&mut current, component)?;
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => validate_plain_directory(&current, &metadata)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match std::fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        validate_plain_directory(&current, &std::fs::symlink_metadata(&current)?)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn ensure_destination_parent_plain(destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("tree destination has no parent directory"))?;
    ensure_plain_directory_tree(parent)
}

/// Promote a fully synced staged regular file while preserving a link-like or
/// directory destination and avoiding a probe/rename race for absent names.
pub(crate) fn promote_staged_replace(staged: &Path, destination: &Path) -> io::Result<()> {
    ensure_destination_parent_plain(destination)?;
    let staged_meta = std::fs::symlink_metadata(staged)?;
    if !staged_meta.is_file() || super::local_platform::metadata_is_link_like(&staged_meta) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "agent staged promotion source must be a regular file",
        ));
    }

    match std::fs::symlink_metadata(destination) {
        Ok(meta) if meta.is_file() && !super::local_platform::metadata_is_link_like(&meta) => {
            super::local_platform::replace_file_atomic(staged, destination)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to replace a directory or link-like destination",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            super::local_platform::rename_no_replace(staged, destination)
        }
        Err(error) => Err(error),
    }
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn push_component(path: &mut PathBuf, component: Component<'_>) -> io::Result<()> {
    match component {
        Component::Prefix(prefix) => path.push(prefix.as_os_str()),
        Component::RootDir => path.push(std::path::MAIN_SEPARATOR_STR),
        Component::CurDir => {}
        Component::Normal(name) => path.push(name),
        Component::ParentDir => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination contains a parent path component",
            ));
        }
    }
    Ok(())
}

fn validate_plain_directory(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    if super::local_platform::metadata_is_link_like(metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "destination ancestor is a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!(
                "destination ancestor is not a directory: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
