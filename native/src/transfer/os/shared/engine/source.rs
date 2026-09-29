//! Change detection on the source side. A local source is observed when it
//! is opened and again after the copy (size, times, and that the path still
//! names the opened file); in a packet before the chunk that completes it. A
//! remote source must deliver exactly the listed length; Drive's listed MD5
//! is checked while streaming, other providers (and server-side copies) get
//! one stat before anything is published (K21).
use super::super::engine_policy::listed_time_differs;
use super::super::walk_listers::native;
use super::ops::OpError;
use super::queue::FileWork;
use crate::vfs::Backend;
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Observation {
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

fn grown(path: &str) -> OpError {
    OpError::source(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{path}: Quelle ist während der Übertragung gewachsen"),
    ))
}

fn changed(path: &str) -> OpError {
    OpError::source(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{path}: Quelle wurde während der Übertragung geändert"),
    ))
}

/// An opened local source file.
pub(crate) struct LocalSource {
    file: File,
    path: PathBuf,
    display: String,
    before: Observation,
}

impl LocalSource {
    pub(crate) fn open(path: &str) -> Result<Self, OpError> {
        let native_path = native(path);
        let metadata =
            crate::local_access::symlink_metadata(&native_path).map_err(OpError::source)?;
        let before = observe(&native_path, &metadata, path)?;
        let file = crate::local_access::open_read(&native_path).map_err(OpError::source)?;
        let source = Self {
            file,
            path: native_path,
            display: path.to_string(),
            before,
        };
        source.verify()?;
        Ok(source)
    }

    pub(crate) fn length(&self) -> u64 {
        self.before.length
    }

    /// The opened file and the path are still what was opened.
    pub(crate) fn verify(&self) -> Result<(), OpError> {
        let opened = self.file.metadata().map_err(OpError::source)?;
        let named = crate::local_access::symlink_metadata(&self.path).map_err(OpError::source)?;
        if observe(&self.path, &opened, &self.display)? != self.before
            || observe(&self.path, &named, &self.display)? != self.before
        {
            return Err(changed(&self.display));
        }
        Ok(())
    }

    /// Next bytes, at most one beyond the opened length so a growing file is
    /// noticed instead of copied without end.
    pub(crate) fn read_bounded(
        &mut self,
        buffer: &mut [u8],
        copied: u64,
    ) -> Result<usize, OpError> {
        let wanted = self
            .before
            .length
            .saturating_sub(copied)
            .saturating_add(1)
            .min(buffer.len() as u64) as usize;
        let read = loop {
            match self.file.read(&mut buffer[..wanted]) {
                Ok(read) => break read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(OpError::source(error)),
            }
        };
        if read as u64 > self.before.length.saturating_sub(copied) {
            return Err(grown(&self.display));
        }
        Ok(read)
    }

    /// The whole content was read: exactly the opened length, unchanged.
    pub(crate) fn complete(&self, copied: u64) -> Result<(), OpError> {
        if copied != self.before.length {
            return Err(changed(&self.display));
        }
        self.verify()
    }

    /// Next bytes of a packet entry after `sent`, never beyond the opened
    /// length. The chunk that completes the entry comes only when the file
    /// ends there and did not change, so a packet never publishes a file
    /// that changed while it was read.
    pub(crate) fn read_entry(&mut self, buffer: &mut [u8], sent: u64) -> Result<usize, OpError> {
        let remaining = self.before.length.saturating_sub(sent);
        let wanted = remaining.min(buffer.len() as u64) as usize;
        let read = loop {
            match self.file.read(&mut buffer[..wanted]) {
                Ok(read) => break read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(OpError::source(error)),
            }
        };
        if read == 0 && wanted > 0 {
            return Err(changed(&self.display));
        }
        if read as u64 == remaining {
            self.check_end()?;
        }
        Ok(read)
    }

    /// The opened length was read: no byte follows, nothing changed.
    fn check_end(&mut self) -> Result<(), OpError> {
        let mut probe = [0u8; 1];
        loop {
            match self.file.read(&mut probe) {
                Ok(0) => return self.verify(),
                Ok(_) => return Err(grown(&self.display)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(OpError::source(error)),
            }
        }
    }
}

fn observe(
    path: &std::path::Path,
    metadata: &Metadata,
    display: &str,
) -> Result<Observation, OpError> {
    if crate::local_access::metadata_is_link_like(path, metadata) || !metadata.is_file() {
        return Err(OpError::source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{display}: keine reguläre Datei (Links, Reparse-Punkte und Spezialdateien werden nicht übertragen)"),
        )));
    }
    Ok(Observation {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    })
}

/// What a remote source must turn out to be after reading.
pub(crate) struct RemoteCheck {
    expected: Option<u64>,
    hasher: Option<md5::Context>,
}

impl RemoteCheck {
    /// `expected` is the backend's read length (`None` for exports, whose
    /// length and hash differ from the listing).
    pub(crate) fn new(file: &FileWork, expected: Option<u64>) -> Self {
        let hasher = (expected.is_some() && file.md5.is_some()).then(md5::Context::new);
        Self { expected, hasher }
    }

    /// Whether the listed MD5 is being checked (a resumed read must feed
    /// the bytes it already has).
    pub(crate) fn hashing(&self) -> bool {
        self.hasher.is_some()
    }

    /// At most this many bytes may still come after `copied`.
    pub(crate) fn limit(&self, copied: u64, buffer: usize) -> usize {
        match self.expected {
            Some(length) => length
                .saturating_sub(copied)
                .saturating_add(1)
                .min(buffer as u64) as usize,
            None => buffer,
        }
    }

    /// Accepts `bytes` read after `copied`; an overlong source is an error.
    pub(crate) fn update(
        &mut self,
        file: &FileWork,
        copied: u64,
        bytes: &[u8],
    ) -> Result<(), OpError> {
        if self
            .expected
            .is_some_and(|length| bytes.len() as u64 > length.saturating_sub(copied))
        {
            return Err(grown(&file.source));
        }
        if let Some(hasher) = self.hasher.as_mut() {
            hasher.consume(bytes);
        }
        Ok(())
    }

    /// Verifies a complete read whose source the peer watched itself
    /// (packets: the peer ends an item with an error when it changed):
    /// length and, when listed, MD5.
    pub(crate) fn finish_streamed(self, file: &FileWork, read: u64) -> Result<(), OpError> {
        if self.expected.is_some_and(|length| length != read) {
            return Err(changed(&file.source));
        }
        if let (Some(hasher), Some(listed)) = (self.hasher, file.md5.as_deref()) {
            if !format!("{:x}", hasher.compute()).eq_ignore_ascii_case(listed) {
                return Err(changed(&file.source));
            }
        }
        Ok(())
    }

    /// Verifies the complete read of `read` bytes; one stat of the source
    /// when no hash could prove it unchanged.
    pub(crate) fn finish(
        self,
        backend: &dyn Backend,
        file: &FileWork,
        read: u64,
    ) -> Result<(), OpError> {
        if self.expected.is_some_and(|length| length != read) {
            return Err(changed(&file.source));
        }
        if let (Some(hasher), Some(listed)) = (self.hasher, file.md5.as_deref()) {
            let digest = format!("{:x}", hasher.compute());
            return if digest.eq_ignore_ascii_case(listed) {
                Ok(())
            } else {
                Err(changed(&file.source))
            };
        }
        unchanged_since_listing(backend, file)
    }
}

/// One stat of a remote source: still the file the listing showed (size,
/// time at the listing's grain, backend id).
pub(crate) fn unchanged_since_listing(
    backend: &dyn Backend,
    file: &FileWork,
) -> Result<(), OpError> {
    let now = backend
        .stat(&file.source)
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => changed(&file.source),
            _ => OpError::source(error),
        })?;
    let same_id = match (&now.id, &file.id) {
        (Some(now), Some(listed)) => now == listed,
        _ => true,
    };
    if now.is_dir
        || now.is_symlink
        || now.size != file.size
        || listed_time_differs(file.mtime_ms, now.mtime_ms)
        || !same_id
    {
        return Err(changed(&file.source));
    }
    Ok(())
}
