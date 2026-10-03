use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::OnceLock;

use super::fs_profile::FlushModel;
use super::local_dirs::{ensure_plain_component, FolderGuard};
use super::local_platform;
use super::local_stage::DurableFile;
use super::{Backend, OmissionReason, Scheme, VfsListing, VfsMeta, VfsOmission, VfsResult};
use crate::local_access::{EntryError, EntryKind, FinalLink, LocalEntry};

// Intentionally duplicated from `scanner.rs` (tiny) to keep this module
// self-contained - isolation over DRY, per the staged remote-layer plan.

#[inline]
fn ms_since_unix(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

fn meta_to_vfs(name: String, path: &Path, meta: &std::fs::Metadata) -> VfsMeta {
    let (hidden, system) = local_platform::local_attrs(meta);
    let class = crate::local_access::metadata_class(path, meta);
    let is_symlink = class.link_like;
    let is_dir = meta.is_dir() && !is_symlink;
    let special = class.special && !is_dir;
    VfsMeta {
        name,
        is_dir,
        is_symlink,
        special,
        size: if is_dir || special { 0 } else { meta.len() },
        mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
        btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
        hidden,
        system,
        id: None,
        content_md5: None,
    }
}

fn unicode_name(name: &OsStr) -> io::Result<String> {
    name.to_str().map(str::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "local filename is not valid Unicode",
        )
    })
}

/// One listed entry; `Err` with its stored name when that name is no path
/// of this interface (not valid Unicode, or one Win32 cannot address).
fn entry_meta(entry: LocalEntry) -> Result<VfsMeta, (OsString, &'static str)> {
    if entry.unreachable {
        return Err((entry.name, "Nicht darstellbarer Dateiname"));
    }
    let Some(name) = entry.name.to_str().map(str::to_owned) else {
        return Err((entry.name, "local filename is not valid Unicode"));
    };
    let special = entry.kind == EntryKind::Other && !entry.is_dir && !entry.is_link_like;
    Ok(VfsMeta {
        name,
        is_dir: entry.is_dir && !entry.is_link_like,
        is_symlink: entry.is_link_like,
        special,
        size: if special { 0 } else { entry.size },
        mtime_ms: entry.mtime_ms,
        btime_ms: entry.btime_ms,
        hidden: entry.hidden,
        system: entry.system,
        id: None,
        content_md5: None,
    })
}

/// `std::fs`-backed local disk using the host path adapter at the boundary.
pub struct LocalBackend {
    root: String, // forward-slash, trailing slash trimmed
    folders: FolderGuard,
    /// Device of the root when its filesystem is flushed as a whole by
    /// `syncfs` (deferred stages on it wait for `sync_filesystem`).
    batched_device: OnceLock<Option<u64>>,
}

impl LocalBackend {
    pub fn new(root: &str) -> Self {
        let r = root.trim().replace('\\', "/");
        let r = r.trim_end_matches('/');
        let root = if r.is_empty() {
            "/".to_string()
        } else {
            r.to_string()
        };
        LocalBackend {
            folders: FolderGuard::new(&root),
            root,
            batched_device: OnceLock::new(),
        }
    }

    pub(super) fn batched_device(&self) -> Option<u64> {
        *self.batched_device.get_or_init(|| {
            let root = local_platform::to_os(&self.root);
            let profile = local_platform::filesystem_profile(&root).ok()?;
            if profile.flush != FlushModel::Batched {
                return None;
            }
            local_platform::device_of(&std::fs::metadata(&root).ok()?)
        })
    }

    /// Lists `path`, leaving out (and naming) entries that cannot be listed:
    /// vanished ones, unreadable ones and names that are no path here. Only
    /// a failure of the enumeration itself fails the listing.
    pub(super) fn list_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let mut listing = VfsListing::default();
        for item in crate::local_access::read_directory(&local_platform::to_os(path))? {
            match item {
                Ok(entry) => match entry_meta(entry) {
                    Ok(meta) => listing.entries.push(meta),
                    Err((name, detail)) => listing.omitted.push(VfsOmission {
                        rel: name.to_string_lossy().into_owned(),
                        reason: OmissionReason::Unrepresentable,
                        detail: detail.to_string(),
                    }),
                },
                Err(error) => {
                    let named =
                        EntryError::name_of(&error).map(|name| name.to_string_lossy().into_owned());
                    let Some(rel) = named else {
                        return Err(error);
                    };
                    listing.omitted.push(VfsOmission {
                        rel,
                        reason: if error.kind() == io::ErrorKind::NotFound {
                            OmissionReason::Vanished
                        } else {
                            OmissionReason::Unreadable
                        },
                        detail: error.to_string(),
                    });
                }
            }
        }
        Ok(listing)
    }

    fn open_regular(&self, path: &str) -> io::Result<std::fs::File> {
        crate::local_access::open_regular(&local_platform::to_os(path), FinalLink::Follow)
    }

    fn create_new(&self, path: &str) -> io::Result<std::fs::File> {
        let path = local_platform::to_os(path);
        local_platform::check_new_name(&path)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }
}

impl Backend for LocalBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }
    fn namespace_identity(&self) -> String {
        "local".into()
    }
    fn extensions(&self) -> Option<&dyn super::BackendExtensions> {
        Some(self)
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let dir = local_platform::to_os(path);
        crate::local_access::read_directory(&dir)?
            .map(|entry| {
                entry_meta(entry?)
                    .map_err(|(_, detail)| io::Error::new(io::ErrorKind::InvalidData, detail))
            })
            .filter(|entry| !matches!(entry, Err(error) if error.kind() == io::ErrorKind::NotFound))
            .collect()
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let p = local_platform::to_os(path);
        let meta = crate::local_access::symlink_metadata(&p)?;
        let name = local_platform::reported_name(&p)
            .as_deref()
            .map(unicode_name)
            .transpose()?
            .unwrap_or_else(|| path.to_string());
        Ok(meta_to_vfs(name, &p, &meta))
    }

    /// Reads regular files only: a FIFO is never waited on and a device never
    /// streamed; a link to a file is followed (Explorer semantics).
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(self.open_regular(path)?))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let path = local_platform::to_os(path);
        local_platform::check_new_name(&path)?;
        Ok(Box::new(std::fs::File::create(path)?))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(self.create_new(path)?))
    }
    /// Sync, mounts and Share hosts: the stage is on stable storage once
    /// its writer is flushed.
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        _size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(DurableFile(self.create_new(path)?)))
    }
    /// Copies whose source stays keep Explorer semantics (no flush).
    fn open_write_copy_stage_unsynced(
        &self,
        path: &str,
        _size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.open_write_new(path)
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let staged = super::promotion::unique_staging_path(self, dst, "copy")?;
        let result = (|| {
            let mut reader = self.open_regular(src)?;
            let permissions = reader.metadata()?.permissions();
            let mut writer = local_platform::create_new_private(&local_platform::to_os(&staged))?;
            let copied = std::io::copy(&mut reader, &mut writer)?;
            writer.flush()?;
            match local_platform::unix_mode(&reader.metadata()?) {
                Some(mut mode) => {
                    match std::fs::symlink_metadata(local_platform::to_os(dst)) {
                        Ok(destination) if destination.is_file() => {
                            mode &= local_platform::unix_mode(&destination).unwrap_or(mode);
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                    if let Err(error) = local_platform::set_unix_mode(&writer, mode & 0o777) {
                        if !matches!(
                            error.kind(),
                            io::ErrorKind::PermissionDenied
                                | io::ErrorKind::Unsupported
                                | io::ErrorKind::InvalidInput
                        ) {
                            return Err(error);
                        }
                    }
                }
                None => writer.set_permissions(permissions)?,
            }
            drop(writer);
            super::promotion::promote_staged_replace(self, &staged, dst)?;
            Ok(copied)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(local_platform::to_os(&staged));
        }
        result
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let destination = local_platform::to_os(dst);
        local_platform::check_new_name(&destination)?;
        std::fs::rename(local_platform::to_os(src), destination)
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let destination = local_platform::to_os(dst);
        local_platform::check_new_name(&destination)?;
        local_platform::rename_no_replace(&local_platform::to_os(src), &destination)
    }
    /// Replacing publishes with the platform's atomic replace (write-through
    /// on Windows, where a read-only destination is replaced as well).
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        super::promotion::promote_staged_with(self, staged, destination, |staged, destination| {
            local_platform::replace_file(
                &local_platform::to_os(staged),
                &local_platform::to_os(destination),
            )
        })
    }
    fn rename_overwrites(&self) -> bool {
        true // std::fs::rename atomically replaces an existing destination
    }
    fn staged_write_capabilities(&self, _root: &str) -> super::StagedWriteCapabilities {
        super::StagedWriteCapabilities::complete()
    }
    fn is_local(&self) -> bool {
        true // a local disk read to hash a file is cheap (no network)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        local_platform::remove_file_like(&local_platform::to_os(path))
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_dir(local_platform::to_os(path))
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.folders.mkdir_all(&local_platform::to_os(path))
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        ensure_plain_component(&local_platform::to_os(path))
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        let path = local_platform::to_os(path);
        local_platform::check_new_name(&path)?;
        std::fs::create_dir(path)
    }
    /// The kernel copies into the new stage: on an SMB share (UNC
    /// connections) the server copies itself (`CopyFile2`: offload/COPYCHUNK),
    /// on NFS/CIFS `copy_file_range` does, so no byte passes this machine.
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        local_platform::check_new_name(&local_platform::to_os(stage))?;
        crate::copy::copy_to_new_file(
            &local_platform::to_os(src),
            &local_platform::to_os(stage),
            size,
            cancel,
        )
        .map(Some)
    }
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        let path = local_platform::to_os(stage);
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || crate::local_access::metadata_is_link_like(&path, &metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "copy stage is not a regular file",
            ));
        }
        std::fs::remove_file(path)
    }
    fn open_read_at(
        &self,
        path: &str,
        _id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        use std::io::{Seek, SeekFrom};
        let mut file = self.open_regular(path)?;
        file.seek(SeekFrom::Start(offset))?;
        Ok(Some(Box::new(file)))
    }
    fn flow_key(&self, path: &str) -> String {
        local_platform::volume_key(path)
    }
}
