//! Change-watch capability of a held directory pin (M14, WIN-SHARE-WATCH).
//!
//! A read pin is synchronous, so `ReadDirectoryChangesW` needs a second,
//! overlapped handle of the same object. `ReOpenFile` derives it from the
//! pin itself (no name is parsed again); only where a provider refuses that,
//! the pinned physical path is opened without following a final link. Either
//! handle must have the pin's identity (volume serial number and file ID),
//! otherwise it is refused: a path that names another object than the pin
//! never becomes the watched root. The pin denies delete sharing, so the root
//! and its ancestors cannot be renamed or replaced while it is held.
use std::{
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, RawHandle},
    },
    path::Path,
};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, ReOpenFile, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOTE;

use super::directory_handle::{identity, DirectoryHandle};
use crate::local_access::protocol::ReadKind;
use crate::local_access::NotRegular;

/// The watch handle shares everything: it must never block a rename or
/// delete inside the root (the pin alone keeps the root in place).
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const WATCH_FLAGS: u32 =
    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED | FILE_FLAG_OPEN_REPARSE_POINT;
/// `ReadDirectoryChangesW` needs `FILE_LIST_DIRECTORY`; the identity and
/// link checks of `finish` query the handle's attributes, so it asks for
/// `FILE_READ_ATTRIBUTES` explicitly (no sharing mode depends on it).
const WATCH_ACCESS: u32 = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES;

/// An overlapped `FILE_LIST_DIRECTORY` handle of a pinned directory. It is
/// only for `ReadDirectoryChangesW` with an `OVERLAPPED`; dropping it closes
/// the handle (cancel pending reads first with `CancelIoEx`).
pub struct WatchDirectory {
    file: File,
    remote: bool,
}

impl WatchDirectory {
    /// The raw overlapped handle for `ReadDirectoryChangesW`.
    pub fn raw_handle(&self) -> RawHandle {
        self.file.as_raw_handle()
    }

    /// A network redirector root: its change buffer must stay at most 64 KiB
    /// and notification completeness is not guaranteed.
    pub fn is_remote(&self) -> bool {
        self.remote
    }
}

impl DirectoryHandle {
    /// Opens the overlapped watch handle of this pin (see the module notes).
    /// Errors: `InvalidData` when the only openable object is not the pinned
    /// one; the provider's error when neither open works.
    pub fn open_watch(&self) -> io::Result<WatchDirectory> {
        let file = match reopen(self.file()) {
            Ok(file) => file,
            // The provider's own error stays the reported one.
            Err(reopen_error) => open_path(self.path()).map_err(|_| reopen_error)?,
        };
        self.finish(file)
    }

    /// The fallback open of `open_watch` for an explicit path, refused unless
    /// that path names the pinned object. Exposed for the Windows task suite.
    #[doc(hidden)]
    pub fn open_watch_at(&self, path: &Path) -> io::Result<WatchDirectory> {
        let file = open_path(path)?;
        self.finish(file)
    }

    fn finish(&self, file: File) -> io::Result<WatchDirectory> {
        if identity::identity(self.file())? != identity::identity(&file)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "watch handle names another object than the held directory pin",
            ));
        }
        if super::directory::classify_open_file(&file)?.link_like {
            return Err(NotRegular::Link.error());
        }
        Ok(WatchDirectory {
            file,
            remote: is_remote(self.path()),
        })
    }

    /// Whether a proper ancestor of `rel` (a `/`-separated path below this
    /// pin) is a link, junction or other redirecting entry, or not a plain
    /// name. Changes there are not content of this root. The entry itself
    /// may be a link: its own creation, rename or removal is root content.
    /// An ancestor that is gone no longer redirects anything (`false`; the
    /// rescan decides). Each component is opened without following it, after
    /// every component before it was found to be no link.
    pub fn under_link(&self, rel: &str) -> bool {
        let mut parts: Vec<&str> = rel.split('/').collect();
        parts.pop();
        let mut path = self.path().to_path_buf();
        for part in parts {
            if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':']) {
                return true;
            }
            path.push(part);
            let Ok(file) = super::read::open_direct(&path, ReadKind::Metadata, SHARE_ALL) else {
                return false;
            };
            match super::directory::classify_open_file(&file) {
                Ok(class) if class.link_like => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }
}

fn reopen(pin: &File) -> io::Result<File> {
    // SAFETY: `pin` is a live handle opened by CreateFileW; the result is
    // checked before it is owned.
    let handle = unsafe { ReOpenFile(pin.as_raw_handle(), WATCH_ACCESS, SHARE_ALL, WATCH_FLAGS) };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh handle owned by nobody else.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_path(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .access_mode(WATCH_ACCESS)
        .share_mode(SHARE_ALL)
        .custom_flags(WATCH_FLAGS)
        .open(path)
}

/// UNC paths and mapped network drives (the pin's physical path).
fn is_remote(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    if text.starts_with(r"\\?\UNC\") || (text.starts_with(r"\\") && !text.starts_with(r"\\?\")) {
        return true;
    }
    let drive = text.trim_start_matches(r"\\?\");
    let mut chars = drive.chars();
    let (Some(letter), Some(':')) = (chars.next(), chars.next()) else {
        return false;
    };
    if !letter.is_ascii_alphabetic() {
        return false;
    }
    let root: Vec<u16> = std::ffi::OsStr::new(&format!("{letter}:\\"))
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: NUL-terminated drive root.
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}
