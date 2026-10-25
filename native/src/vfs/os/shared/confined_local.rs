//! Local vault storage with a retained root and descriptor-relative children.
//! It accepts the existing absolute VFS paths, but never reopens those paths.
use std::{ffi::OsStr, fs::File, io::{self, Read, Write, Seek, SeekFrom}, sync::atomic::AtomicBool};
use crate::local_access::{ConfinedDirectory, EntryError};
use super::{Backend, BackendExtensions, Scheme, VfsMeta, VfsResult, RootConfinement, StagedWriteCapabilities, OmissionReason, promote_staged_with, unique_staging_path};
use super::confined_local_platform as platform;

pub(crate) struct ConfinedLocalBackend {
    pub(super) root: ConfinedDirectory,
    prefix: String,
    identity: String,
    display: String,
    flow: String,
}

impl ConfinedLocalBackend {
    pub(crate) fn open(root: &str) -> io::Result<Self> {
        let held = ConfinedDirectory::open_root(&super::local_platform::to_os(root))?;
        platform::qualify(&held)?;
        let original = super::LocalBackend::new(root);
        Ok(Self { root: held, prefix: root.trim_end_matches('/').to_string(), identity: original.state_identity(), display: original.root_display(), flow: original.flow_key(root) })
    }
    pub(super) fn restrict(&self, root: &str) -> io::Result<Self> {
        Ok(Self { root: self.directory(root)?, prefix: root.trim_end_matches('/').into(), identity: super::LocalBackend::new(root).state_identity(), display: super::LocalBackend::new(root).root_display(), flow: self.flow.clone() })
    }
    pub(super) fn parts<'a>(&self, path: &'a str) -> io::Result<Vec<&'a str>> {
        let relative = if path.trim_end_matches('/') == self.prefix { "" }
        else if self.prefix.is_empty() { path.strip_prefix('/').ok_or_else(boundary)? }
        else { path.strip_prefix(&self.prefix).and_then(|rest| rest.strip_prefix('/')).ok_or_else(boundary)? };
        let mut result = Vec::new();
        for name in relative.split('/').filter(|name| !name.is_empty()) {
            if name == "." || name == ".." || name.contains('\0') { return Err(boundary()); }
            result.push(name);
        }
        Ok(result)
    }
    pub(super) fn directory(&self, path: &str) -> io::Result<ConfinedDirectory> {
        let mut directory = self.root.clone();
        for name in self.parts(path)? { directory = directory.child(OsStr::new(name))?; }
        Ok(directory)
    }
    pub(super) fn parent(&self, path: &str) -> io::Result<(ConfinedDirectory, String)> {
        let mut parts = self.parts(path)?;
        let name = parts.pop().ok_or_else(boundary)?.to_string();
        let mut directory = self.root.clone();
        for part in parts { directory = directory.child(OsStr::new(part))?; }
        Ok((directory, name))
    }
    pub(super) fn entry(&self, path: &str) -> io::Result<File> {
        if self.parts(path)?.is_empty() { return self.root.file().try_clone(); }
        let (parent, name) = self.parent(path)?;
        parent.entry(OsStr::new(&name))
    }
    pub(super) fn file(&self, path: &str, writable: bool, exclusive: bool, truncate: bool) -> io::Result<HeldFile> {
        if writable { super::local_platform::check_new_name(&super::local_platform::to_os(path))?; }
        let (parent, name) = self.parent(path)?;
        let file = parent.open_file(OsStr::new(&name), writable, exclusive, truncate, writable)?;
        Ok(HeldFile { file, _parent: parent, _root: self.root.clone(), durable: false, expected: None, written: 0, failed: None })
    }
    pub(super) fn existing_file(&self, path: &str, writable: bool) -> io::Result<HeldFile> {
        let (parent, name) = self.parent(path)?;
        let file = parent.open_file(OsStr::new(&name), writable, false, false, false)?;
        Ok(HeldFile { file, _parent: parent, _root: self.root.clone(), durable: false, expected: None, written: 0, failed: None })
    }
    pub(super) fn sized_stage(&self, path: &str, size: u64, durable: bool) -> io::Result<HeldFile> {
        let mut file = self.file(path, true, true, false)?;
        file.expected = Some(size);
        file.durable = durable;
        Ok(file)
    }
    fn rename_inner(&self, source: &str, destination: &str, replace: bool) -> io::Result<()> {
        super::local_platform::check_new_name(&super::local_platform::to_os(destination))?;
        let (from, name) = self.parent(source)?;
        let (to, target) = self.parent(destination)?;
        from.rename(OsStr::new(&name), &to, OsStr::new(&target), replace)
    }
    pub(super) fn tolerant(&self, path: &str) -> VfsResult<super::VfsListing> {
        let directory = self.directory(path)?;
        let mut result = super::VfsListing::default();
        for entry in directory.list()? {
            match entry {
                Ok(entry) => {
                    let Some(name) = entry.name.to_str().map(str::to_string).filter(|_| !entry.unreachable) else {
                        result.omitted.push(super::VfsOmission { rel: entry.name.to_string_lossy().into_owned(), reason: super::OmissionReason::Unrepresentable, detail: "local filename is not representable".into() });
                        continue;
                    };
                    // Re-observe on the checked child handle: a directory
                    // enumerator must not leak metadata through a bind mount.
                    match directory.entry(&entry.name).and_then(|file| metadata(&file, name.clone())) {
                        Ok(mut meta) => { meta.hidden |= name.starts_with('.'); result.entries.push(meta); },
                        Err(error) => result.omitted.push(super::VfsOmission { rel: name, reason: if error.kind() == io::ErrorKind::NotFound { super::OmissionReason::Vanished } else { super::OmissionReason::Unreadable }, detail: error.to_string() }),
                    }
                },
                Err(error) => {
                    let name = EntryError::name_of(&error).ok_or_else(|| io::Error::new(error.kind(), error.to_string()))?.to_string_lossy().into_owned();
                    result.omitted.push(super::VfsOmission { rel: name, reason: if error.kind() == io::ErrorKind::NotFound { super::OmissionReason::Vanished } else { super::OmissionReason::Unreadable }, detail: error.to_string() });
                }
            }
        }
        Ok(result)
    }
}

pub(super) struct HeldFile {
    pub(super) file: File,
    _parent: ConfinedDirectory,
    _root: ConfinedDirectory,
    durable: bool,
    expected: Option<u64>,
    written: u64,
    failed: Option<io::ErrorKind>,
}
impl Read for HeldFile { fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> { self.file.read(bytes) } }
impl Write for HeldFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.failed { return Err(io::Error::new(kind, "local stage previously failed")); }
        if self.expected.is_some_and(|size| bytes.len() as u64 > size.saturating_sub(self.written)) {
            self.failed = Some(io::ErrorKind::InvalidData);
            return Err(io::Error::new(io::ErrorKind::InvalidData, "local stage exceeds declared size"));
        }
        match self.file.write(bytes) {
            Ok(written) => { self.written = self.written.saturating_add(written as u64); Ok(written) },
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => { self.failed = Some(error.kind()); Err(error) },
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(kind) = self.failed { return Err(io::Error::new(kind, "local stage previously failed")); }
        if self.expected.is_some_and(|size| self.written != size) { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "local stage has not received its declared size")); }
        self.file.flush()?;
        if self.durable { self.file.sync_all()?; }
        Ok(())
    }
}

#[path = "confined_local_backend.rs"]
mod backend;

fn boundary() -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, "path is outside the retained local vault root") }
fn metadata(file: &File, name: String) -> io::Result<VfsMeta> {
    let meta = file.metadata()?;
    let class = crate::local_access::classify_open_file(file)?;
    let (hidden, system) = super::local_platform::local_attrs(&meta);
    Ok(VfsMeta { name, is_dir: meta.is_dir() && !class.link_like, is_symlink: class.link_like, special: class.special,
        size: if meta.is_dir() || class.special { 0 } else { meta.len() }, mtime_ms: meta.modified().map(crate::local_access::system_time_ms).unwrap_or(0),
        btime_ms: meta.created().map(crate::local_access::system_time_ms).unwrap_or(0), hidden, system, id: None, content_md5: None })
}
