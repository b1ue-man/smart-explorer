//! Desktop folders on Windows (DSK-M5, C1.2): the user's desktop
//! (`FOLDERID_Desktop`, also when it is redirected, for example to OneDrive)
//! and the all-users desktop (`FOLDERID_PublicDesktop`; renaming or deleting
//! there fails without administrator rights with the system's message). The
//! paths come from the system's known-folder registration, never from the
//! process environment. An entry with the attribute "Versteckt" or "System"
//! (`desktop.ini`) is never shown, as in Explorer's default view.
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows_sys::core::GUID;
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_PublicDesktop, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

use crate::desktop::os::feed_sources::DesktopFolder;

/// A known folder of this user; `None` when Windows names none.
pub(crate) fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut path: *mut u16 = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut path)
    };
    let mut units = Vec::new();
    if !path.is_null() {
        // SAFETY: the shell returns a NUL-terminated string it allocated.
        unsafe {
            let mut at = path;
            while *at != 0 {
                units.push(*at);
                at = at.add(1);
            }
            CoTaskMemFree(path as *const std::ffi::c_void);
        }
    }
    if result < 0 || units.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&units)))
}

fn same_folder(left: &Path, right: &Path) -> bool {
    let normalized = |path: &Path| path.to_string_lossy().trim_end_matches('\\').to_lowercase();
    normalized(left) == normalized(right)
}

/// The user's desktop, then the all-users desktop; a folder that does not
/// exist or equals the first is left out.
pub(crate) fn desktop_folders() -> Vec<DesktopFolder> {
    let mut folders: Vec<DesktopFolder> = Vec::new();
    for (id, public) in [(&FOLDERID_Desktop, false), (&FOLDERID_PublicDesktop, true)] {
        let Some(path) = known_folder(id).filter(|path| path.is_dir()) else {
            continue;
        };
        if folders
            .iter()
            .any(|folder| same_folder(&folder.path, &path))
        {
            continue;
        }
        folders.push(DesktopFolder {
            path,
            public,
            launchers: false,
        });
    }
    folders
}

/// The entry carries "Versteckt" or "System" (read from the folder listing,
/// a link is not followed).
pub(crate) fn hidden_entry(entry: &std::fs::DirEntry) -> bool {
    entry.metadata().is_ok_and(|meta| {
        meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
    })
}
