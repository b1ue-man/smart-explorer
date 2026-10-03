//! Descriptor-relative traversal. Only the chosen root follows links;
//! opening a child and obtaining its metadata stays on its parent handle.
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::{Component, Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;

use crate::local_access::{EntryError, EntryKind, LocalEntry, NotRegular};

#[path = "quarantine.rs"]
mod quarantine;
pub(crate) use quarantine::QuarantinedChild;
#[path = "create.rs"]
mod create;
#[path = "private_ancestors.rs"]
mod private_ancestors;
#[path = "remove.rs"]
mod remove;

#[derive(Clone)]
pub(crate) struct DirectoryHandle {
    file: Arc<File>,
    path: PathBuf,
}

impl DirectoryHandle {
    /// Metadata of this opened directory, even after a path or ancestor swap.
    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.file.metadata()
    }

    pub(crate) fn secure_private(&self) -> io::Result<()> {
        super::secure_private_handle(&self.file, true)
    }

    /// Linux watcher anchor for this opened object. The watcher must keep a
    /// clone of this handle alive throughout registration and event delivery;
    /// the descriptor spelling is never a freely reusable child path.
    pub(crate) fn watch_path(&self) -> Option<PathBuf> {
        Some(PathBuf::from(format!(
            "/proc/self/fd/{}/.",
            self.file.as_raw_fd()
        )))
    }

    /// The selected root (and its ancestors) may contain links. Thereafter
    /// every child operation is relative to this opened directory.
    pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)?;
        Ok(Self {
            file: Arc::new(file),
            path: path.to_path_buf(),
        })
    }

    /// Unix has no elevated read broker; consent preserves ordinary rights.
    pub(crate) fn open_root_consented(path: &Path) -> io::Result<Self> {
        Self::open_root(path)
    }

    pub(crate) fn open_child(&self, name: &OsStr) -> io::Result<Self> {
        validate_name(name)?;
        let file = self
            .open_at(name, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .map_err(|error| {
                if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
                    && self
                        .metadata_at(name)
                        .is_ok_and(|metadata| metadata.is_symlink())
                {
                    NotRegular::Link.error()
                } else {
                    error
                }
            })?;
        Ok(Self {
            file: Arc::new(file),
            path: self.path.join(name),
        })
    }

    /// A fresh stream has its own position, so repeated or concurrent scans
    /// of this handle cannot skip each other's directory entries.
    pub(crate) fn read_directory(&self) -> io::Result<DirectoryEntries> {
        let stream_file = self.open_at(OsStr::new("."), libc::O_RDONLY | libc::O_DIRECTORY)?;
        // SAFETY: a live readable directory descriptor, owned by stream_file.
        let stream = unsafe { libc::fdopendir(stream_file.as_raw_fd()) };
        let stream = NonNull::new(stream).ok_or_else(io::Error::last_os_error)?;
        // fdopendir owns the descriptor only on success; its DIR closes it.
        let _descriptor = stream_file.into_raw_fd();
        Ok(DirectoryEntries {
            stream,
            directory: self.clone(),
            ended: false,
        })
    }

    /// Reads a listed file without following a replaced final link. The
    /// metadata-only O_PATH probe avoids opening device nodes for data.
    pub(crate) fn open_regular_child(&self, name: &OsStr) -> io::Result<File> {
        validate_name(name)?;
        super::regular_or_refusal(self.metadata_at(name)?.file_type())?;
        let file = self
            .open_at(name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NOCTTY)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ELOOP) => NotRegular::Link.error(),
                Some(libc::ENXIO) => NotRegular::Special.error(),
                _ => error,
            })?;
        super::regular_or_refusal(file.metadata()?.file_type())?;
        Ok(file)
    }

    fn metadata_at(&self, name: &OsStr) -> io::Result<std::fs::Metadata> {
        self.open_at(name, libc::O_PATH | libc::O_NOFOLLOW)?
            .metadata()
    }

    fn open_at(&self, name: &OsStr, flags: i32) -> io::Result<File> {
        let name = CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name contains NUL"))?;
        // SAFETY: the parent descriptor and C string stay live throughout;
        // these calls never set O_CREAT, so no variadic mode is required.
        let descriptor = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: the new descriptor belongs solely to this File.
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }
}

fn validate_name(name: &OsStr) -> io::Result<()> {
    let mut components = Path::new(name).components();
    if matches!(components.next(), Some(Component::Normal(component)) if component == name)
        && components.next().is_none()
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "entry name is not one child component",
        ))
    }
}

pub(crate) struct DirectoryEntries {
    stream: NonNull<libc::DIR>,
    directory: DirectoryHandle,
    ended: bool,
}

// SAFETY: each stream has one owner and is accessed only via &mut self.
// Moving it between threads never permits concurrent readdir on that DIR.
unsafe impl Send for DirectoryEntries {}

impl Iterator for DirectoryEntries {
    type Item = io::Result<LocalEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }
        loop {
            reset_errno();
            // SAFETY: this instance exclusively owns the still-open stream.
            let entry = unsafe { libc::readdir(self.stream.as_ptr()) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                self.ended = true;
                return if error.raw_os_error() == Some(0) {
                    None
                } else {
                    Some(Err(error))
                };
            }
            // SAFETY: readdir's NUL-terminated name stays valid until the next
            // call on this stream. Copy it before obtaining more metadata.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = OsString::from_vec(bytes.to_vec());
            return Some(
                self.directory
                    .metadata_at(&name)
                    .map(|metadata| local_entry(name.clone(), &metadata))
                    .map_err(|error| {
                        EntryError::wrap(name.clone(), &self.directory.path.join(&name), error)
                    }),
            );
        }
    }
}

impl Drop for DirectoryEntries {
    fn drop(&mut self) {
        // SAFETY: owns this DIR and its descriptor; called exactly once.
        unsafe {
            libc::closedir(self.stream.as_ptr());
        }
    }
}

#[cfg(target_os = "android")]
fn reset_errno() {
    // SAFETY: bionic returns this thread's valid errno storage.
    unsafe {
        *libc::__errno() = 0;
    }
}

#[cfg(not(target_os = "android"))]
fn reset_errno() {
    // SAFETY: glibc/musl returns this thread's valid errno storage.
    unsafe {
        *libc::__errno_location() = 0;
    }
}

fn local_entry(name: OsString, metadata: &std::fs::Metadata) -> LocalEntry {
    let kind = if metadata.is_symlink() {
        EntryKind::Link
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let hidden = name.as_bytes().first() == Some(&b'.');
    LocalEntry {
        name,
        kind,
        is_dir: kind == EntryKind::Directory,
        is_link_like: kind == EntryKind::Link,
        size: if kind == EntryKind::File {
            metadata.len()
        } else {
            0
        },
        mtime_ms: metadata
            .modified()
            .map(crate::local_access::system_time_ms)
            .unwrap_or(0),
        btime_ms: metadata
            .created()
            .map(crate::local_access::system_time_ms)
            .unwrap_or(0),
        hidden,
        system: false,
        unreachable: false,
    }
}
