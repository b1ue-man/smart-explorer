//! Vault I/O relative to retained directory descriptors. No leaf path follows links.
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::{ffi::{OsStrExt, OsStringExt}, fs::MetadataExt, io::{AsRawFd, FromRawFd, IntoRawFd}};
use std::path::Path;
use crate::local_access::{DirectoryHandle, LocalEntry};

#[derive(Clone)]
pub(crate) struct ConfinedDirectory {
    held: DirectoryHandle,
    mount: Option<u64>,
}

impl ConfinedDirectory {
    pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
        let held = DirectoryHandle::open_root(path)?;
        let mount = match mount_id(held.directory_file()) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
            Err(error) => return Err(error),
        };
        Ok(Self { held, mount })
    }
    pub(crate) fn verify_reopen_support(&self) -> io::Result<()> {
        let anchor = CString::new(format!("/proc/self/fd/{}", self.file().as_raw_fd())).map_err(|_| boundary())?;
        let descriptor = unsafe { libc::open(anchor.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if descriptor < 0 { return Err(io::Error::new(io::ErrorKind::Unsupported, format!("descriptor reopen unavailable: {}", io::Error::last_os_error()))); }
        let reopened = unsafe { File::from_raw_fd(descriptor) };
        if identity(&reopened)? != identity(self.file())? { return Err(boundary()); }
        if let Some(expected) = self.mount { if mount_id(&reopened)? != expected { return Err(boundary()); } }
        Ok(())
    }
    pub(crate) fn has_mount_identity(&self) -> bool { self.mount.is_some() }
    pub(crate) fn file(&self) -> &File { self.held.directory_file() }
    pub(crate) fn child(&self, name: &OsStr) -> io::Result<Self> {
        component(name)?;
        let held = self.held.open_child(name)?;
        if let Some(expected) = self.mount { if mount_id(held.directory_file())? != expected { return Err(boundary()); } }
        Ok(Self { held, mount: self.mount })
    }
    pub(crate) fn list(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<LocalEntry>>>> {
        let descriptor = unsafe { libc::openat(self.file().as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if descriptor < 0 { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let stream = unsafe { libc::fdopendir(file.as_raw_fd()) };
        let stream = std::ptr::NonNull::new(stream).ok_or_else(io::Error::last_os_error)?;
        let _ = file.into_raw_fd(); // fdopendir owns this descriptor now.
        Ok(Box::new(Names { stream, _directory: self.clone(), ended: false }))
    }
    pub(crate) fn entry(&self, name: &OsStr) -> io::Result<File> {
        self.open_at(name, libc::O_PATH | libc::O_NOFOLLOW, 0)
    }
    /// A normal LocalBackend request may explicitly select a mount root.
    /// Confined VFS traversal never uses these two root-selection methods.
    pub(crate) fn selected_entry(&self, name: &OsStr) -> io::Result<File> {
        self.open_at_checked(name, libc::O_PATH | libc::O_NOFOLLOW, 0, false)
    }
    pub(crate) fn select_child_root(&self, name: &OsStr) -> io::Result<Self> {
        component(name)?;
        let held = self.held.open_child(name)?;
        let mount = match mount_id(held.directory_file()) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
            Err(error) => return Err(error),
        };
        Ok(Self { held, mount })
    }
    pub(crate) fn open_file(&self, name: &OsStr, writable: bool, exclusive: bool, truncate: bool, create: bool) -> io::Result<File> {
        let selected = if exclusive {
            self.open_at(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW, 0o600)?
        } else {
            match self.entry(name) {
                Ok(selected) => {
                    let metadata = selected.metadata()?;
                    if !metadata.is_file() || (writable && metadata.nlink() != 1) { return Err(boundary()); }
                    // Reopen this held inode, never the replaceable child name.
                    // A device substituted after the metadata probe is never opened.
                    let anchor = CString::new(format!("/proc/self/fd/{}", selected.as_raw_fd())).map_err(|_| boundary())?;
                    let flags = if writable { libc::O_RDWR } else { libc::O_RDONLY };
                    let descriptor = unsafe { libc::open(anchor.as_ptr(), flags | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOCTTY) };
                    if descriptor < 0 { return Err(io::Error::last_os_error()); }
                    let opened = unsafe { File::from_raw_fd(descriptor) };
                    if identity(&selected)? != identity(&opened)? { return Err(boundary()); }
                    if let Some(expected) = self.mount { if mount_id(&opened)? != expected { return Err(boundary()); } }
                    opened
                }
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    // Exclusive creation never opens a special file inserted in
                    // the meantime. A concurrent creator is reported, not adopted.
                    self.open_at(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW, 0o600)?
                }
                Err(error) => return Err(error),
            }
        };
        let meta = selected.metadata()?;
        if !meta.is_file() || (writable && meta.nlink() != 1) { return Err(boundary()); }
        if truncate { selected.set_len(0)?; }
        Ok(selected)
    }
    pub(crate) fn create_dir(&self, name: &OsStr) -> io::Result<Self> {
        let name_c = component(name)?;
        // SAFETY: a live parent FD and one validated child; mode is filtered by umask.
        if unsafe { libc::mkdirat(self.file().as_raw_fd(), name_c.as_ptr(), 0o777) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.child(name)
    }
    pub(crate) fn remove(&self, name: &OsStr, directory: bool) -> io::Result<()> {
        let name = component(name)?;
        // unlinkat never follows the final child; AT_REMOVEDIR cannot recurse.
        if unsafe { libc::unlinkat(self.file().as_raw_fd(), name.as_ptr(), if directory { libc::AT_REMOVEDIR } else { 0 }) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(crate) fn rename(&self, name: &OsStr, target: &Self, target_name: &OsStr, replace: bool) -> io::Result<()> {
        let source = component(name)?;
        let destination = component(target_name)?;
        // SAFETY: both parents are live retained descriptors; final names are
        // renamed as entries, never dereferenced, including swapped links.
        let result = unsafe { libc::syscall(libc::SYS_renameat2, self.file().as_raw_fd(), source.as_ptr(), target.file().as_raw_fd(), destination.as_ptr(), if replace { 0u32 } else { 1u32 }) };
        if result != 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
    pub(crate) fn stable_identity(&self) -> io::Result<Option<String>> {
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(self.file().as_raw_fd(), stat.as_mut_ptr()) } != 0 { return Err(io::Error::last_os_error()); }
        let kind = unsafe { stat.assume_init() }.f_type as u64 & 0xffff_ffff;
        // FAT/exFAT directory positions, arbitrary FUSE identifiers and an
        // unqualified overlay copy-up are not durable rename ownership proofs.
        if matches!(kind, 0x4d44 | 0x2011_bab0 | 0x6573_5546 | 0x794c_7630) { return Ok(None); }
        self.identity().map(Some)
    }
    pub(crate) fn identity(&self) -> io::Result<String> {
        let object = identity(self.file())?;
        Ok(match self.mount { Some(mount) => format!("unix-mounted:{mount}:{object}"), None => object })
    }
    fn open_at(&self, name: &OsStr, flags: i32, mode: libc::mode_t) -> io::Result<File> {
        self.open_at_checked(name, flags, mode, true)
    }
    fn open_at_checked(&self, name: &OsStr, flags: i32, mode: libc::mode_t, same_mount: bool) -> io::Result<File> {
        let name = component(name)?;
        // Nonblocking open cannot hang on a swapped FIFO; the opened type is
        // checked before any data or truncation. O_NOFOLLOW forbids leaf links.
        let fd = unsafe { libc::openat(self.file().as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOCTTY, mode) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_fd(fd) };
        if same_mount { if let Some(expected) = self.mount { if mount_id(&file)? != expected { return Err(boundary()); } } }
        Ok(file)
    }
}

fn identity(file: &File) -> io::Result<String> {
    let meta = file.metadata()?;
    Ok(format!("unix:{}:{}", meta.dev(), meta.ino()))
}

fn component(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') { return Err(boundary()); }
    CString::new(bytes).map_err(|_| boundary())
}
fn boundary() -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, "local root boundary or regular-file identity changed") }

/// Linux's fixed statx UAPI is 256 bytes; stx_mnt_id is at byte 144.
/// Requiring the returned mask prevents a fabricated proof on older kernels.
fn mount_id(file: &File) -> io::Result<u64> {
    #[repr(C, align(8))]
    struct Statx([u8; 256]);
    let mut result = Statx([0; 256]);
    let status = unsafe { libc::syscall(libc::SYS_statx, file.as_raw_fd(), c"".as_ptr(), libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW, 0x1000u32, result.0.as_mut_ptr()) };
    if status != 0 {
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "kernel does not report mount identities"));
        }
        return Err(error);
    }
    let mask = u32::from_ne_bytes(result.0[..4].try_into().map_err(|_| boundary())?);
    if mask & 0x1000 == 0 { return Err(io::Error::new(io::ErrorKind::Unsupported, "kernel does not report the selected mount identity")); }
    Ok(u64::from_ne_bytes(result.0[144..152].try_into().map_err(|_| boundary())?))
}

// Enumerate literal names only. Metadata is read by the VFS caller after
// opening the child relative to the retained parent and checking mount ID.
struct Names {
    stream: std::ptr::NonNull<libc::DIR>,
    _directory: ConfinedDirectory,
    ended: bool,
}
impl Iterator for Names {
    type Item = io::Result<LocalEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.ended { return None; }
        loop {
            #[cfg(target_os = "android")]
            let errno = unsafe { libc::__errno() };
            #[cfg(not(target_os = "android"))]
            let errno = unsafe { libc::__errno_location() };
            unsafe { *errno = 0; }
            let entry = unsafe { libc::readdir(self.stream.as_ptr()) };
            if entry.is_null() {
                self.ended = true;
                let code = unsafe { *errno };
                return if code == 0 { None } else { Some(Err(io::Error::from_raw_os_error(code))) };
            }
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." { continue; }
            return Some(Ok(LocalEntry { name: OsString::from_vec(bytes.to_vec()), ..LocalEntry::default() }));
        }
    }
}
impl Drop for Names { fn drop(&mut self) { unsafe { libc::closedir(self.stream.as_ptr()); } } }
