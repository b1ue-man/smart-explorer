//! Windows launch of a local media file together with the neighbors of its
//! folder, so the viewer's previous/next works as when Explorer opens it
//! (decision: `app/core/media_launch_plan.rs`). Every failure falls back to the
//! plain `ShellExecute` launch.

use super::super::media_launch_plan::{
    newest_major_version, plan_media_launch, DefaultHandler, MediaLaunch, PHOTOS_FAMILY,
};
use crate::types::media_kind_of_name;
use std::path::Path;
use std::ptr::{null, null_mut};

/// Opens the local file `path` (Windows form) in its default viewer with the
/// neighbors of its folder. `false`: nothing was started; the caller launches
/// the plain path.
pub(super) fn open_media_with_neighbors(path: &str) -> bool {
    if media_kind_of_name(path).is_none() {
        return false;
    }
    let Some(ext) = Path::new(path).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let handler = default_handler(ext);
    let is_dir = Path::new(path).is_dir();
    match plan_media_launch(path, is_dir, &handler, photos_major_version) {
        MediaLaunch::Shell => false,
        MediaLaunch::PhotosViewer(uri) => shell_execute(&uri),
        MediaLaunch::NeighborQuery => spawn_neighbor_launch(path.to_string()),
    }
}

/// The user's default handler of `.ext` (Windows 10+ association facts).
fn default_handler(ext: &str) -> DefaultHandler {
    use windows_sys::Win32::UI::Shell::{ASSOCSTR_APPID, ASSOCSTR_PROGID};
    DefaultHandler {
        app_id: assoc_string(ext, ASSOCSTR_APPID),
        prog_id: assoc_string(ext, ASSOCSTR_PROGID),
    }
}

fn assoc_string(ext: &str, kind: windows_sys::Win32::UI::Shell::ASSOCSTR) -> Option<String> {
    use windows_sys::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_NOTRUNCATE};
    let assoc: Vec<u16> = format!(".{ext}").encode_utf16().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_NOTRUNCATE,
            kind,
            assoc.as_ptr(),
            null(),
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if result != 0 {
        return None;
    }
    let end = buffer.iter().position(|&unit| unit == 0)?;
    String::from_utf16(&buffer[..end])
        .ok()
        .filter(|value| !value.is_empty())
}

/// Newest installed Photos package generation of the current user.
fn photos_major_version() -> Option<u32> {
    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows_sys::Win32::Storage::Packaging::Appx::GetPackagesByPackageFamily;
    let family: Vec<u16> = PHOTOS_FAMILY.encode_utf16().chain(Some(0)).collect();
    let mut count = 0u32;
    let mut length = 0u32;
    let sizing = unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            null_mut(),
            &mut length,
            null_mut(),
        )
    };
    if sizing != ERROR_INSUFFICIENT_BUFFER || count == 0 || length == 0 {
        return None;
    }
    let mut names: Vec<*mut u16> = vec![null_mut(); count as usize];
    let mut buffer = vec![0u16; length as usize];
    let filled = unsafe {
        GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            names.as_mut_ptr(),
            &mut length,
            buffer.as_mut_ptr(),
        )
    };
    if filled != ERROR_SUCCESS {
        return None;
    }
    // The full names are the NUL-terminated strings packed into `buffer`.
    let full_names: Vec<String> = buffer
        .split(|&unit| unit == 0)
        .filter(|name| !name.is_empty())
        .filter_map(|name| String::from_utf16(name).ok())
        .collect();
    newest_major_version(full_names.iter().map(String::as_str))
}

/// `ShellExecuteW` of a path or URI; `true` when Windows reports a start.
fn shell_execute(target: &str) -> bool {
    let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            null_mut(),
            null(),
            wide.as_ptr(),
            null(),
            null(),
            1,
        )
    };
    result as isize > 32
}

/// Runs the WinRT launch off the GUI thread: the storage lookups block until
/// Windows answers, which can take a while on network folders.
fn spawn_neighbor_launch(path: String) -> bool {
    std::thread::Builder::new()
        .name("media-open".into())
        .spawn(move || {
            let launched = {
                let _apartment = Apartment::enter(COM_MTA);
                launch_with_neighbors(&path).unwrap_or(false)
            };
            if !launched {
                // Shell extensions behind ShellExecute expect an STA.
                let _apartment = Apartment::enter(COM_STA);
                shell_execute(&path);
            }
        })
        .is_ok()
}

/// `Launcher.LaunchFileWithOptionsAsync` with a shallow, name-sorted
/// neighboring-files query over the parent folder (the query Explorer hands
/// to a file activation).
fn launch_with_neighbors(path: &str) -> windows::core::Result<bool> {
    use windows::core::{Interface, HSTRING};
    use windows::Storage::Search::{FolderDepth, IndexerOption, QueryOptions, SortEntry};
    use windows::Storage::{IStorageFile, StorageFile, StorageFolder};
    use windows::System::{Launcher, LauncherOptions};

    let Some(parent) = Path::new(path).parent() else {
        return Ok(false);
    };
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.get()?;
    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(parent))?.get()?;
    let query = QueryOptions::new()?;
    query.SetFolderDepth(FolderDepth::Shallow)?;
    query.SetIndexerOption(IndexerOption::UseIndexerWhenAvailable)?;
    query.SortOrder()?.Append(&SortEntry {
        PropertyName: HSTRING::from("System.ItemNameDisplay"),
        AscendingOrder: true,
    })?;
    if !folder.AreQueryOptionsSupported(&query)? {
        query.SortOrder()?.Clear()?;
    }
    let options = LauncherOptions::new()?;
    options.SetNeighboringFilesQuery(&folder.CreateFileQueryWithOptions(&query)?)?;
    // windows 0.58 offers the StorageFile → IStorageFile conversion only with
    // the `Storage_Streams` feature; QueryInterface gives the same interface.
    let file: IStorageFile = file.cast()?;
    Launcher::LaunchFileWithOptionsAsync(&file, &options)?.get()
}

const COM_MTA: u32 = windows_sys::Win32::System::Com::COINIT_MULTITHREADED as u32;
const COM_STA: u32 = (windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED
    | windows_sys::Win32::System::Com::COINIT_DISABLE_OLE1DDE) as u32;

/// COM apartment of the current thread for the lifetime of the value.
struct Apartment {
    initialized: bool,
}

impl Apartment {
    fn enter(mode: u32) -> Self {
        let result = unsafe { windows_sys::Win32::System::Com::CoInitializeEx(null(), mode) };
        // S_OK and S_FALSE both need a matching CoUninitialize.
        Self {
            initialized: result == 0 || result == 1,
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    }
}
