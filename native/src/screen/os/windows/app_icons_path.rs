//! Shell icons of local files, folders and shortcuts for the se Desktop
//! surface's tiles (user report 2026-10-27: the surface showed own symbols
//! instead of the real app and Windows icons).
use super::{icon_rgba, DestroyIcon, SHGetFileInfoW, ShellFileInfo, SIDE};

/// The Shell icon of a local file, folder or shortcut as Explorer shows it
/// (a `.lnk` shows its target program's icon), as square straight-alpha
/// RGBA with its side length. For the se Desktop surface's tiles.
pub(crate) fn path_icon_rgba(path: &std::path::Path) -> Option<(u32, Vec<u8>)> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut info = ShellFileInfo {
        icon: std::ptr::null_mut(),
        index: 0,
        attributes: 0,
        display_name: [0; 260],
        type_name: [0; 80],
    };
    // SHGFI_ICON|SHGFI_LARGEICON returns one caller-owned icon.
    let result = unsafe {
        SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<ShellFileInfo>() as u32,
            0x100,
        )
    };
    if result == 0 || info.icon.is_null() {
        return None;
    }
    let rgba = icon_rgba(info.icon);
    unsafe {
        DestroyIcon(info.icon);
    }
    rgba.map(|rgba| (SIDE as u32, rgba))
}

/// `SHSTOCKICONINFO`.
#[repr(C)]
struct StockIconInfo {
    size: u32,
    icon: super::RawHandle,
    system_index: i32,
    icon_index: i32,
    path: [u16; 260],
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetStockIconInfo(id: i32, flags: u32, info: *mut StockIconInfo) -> i32;
}

/// Windows' stock icon for a freedesktop icon name (`SHSTOCKICONID`).
fn stock_id(name: &str) -> Option<i32> {
    Some(match name {
        "computer" | "video-display" => 94, // SIID_DESKTOPPC
        "user-trash" => 31,                 // SIID_RECYCLER
        "user-trash-full" => 32,            // SIID_RECYCLERFULL
        "folder" => 3,                      // SIID_FOLDER
        "folder-open" | "folder-unlocked" => 4, // SIID_FOLDEROPEN
        "folder-locked" => 47,              // SIID_LOCK
        "network-workgroup" => 17,          // SIID_MYNETWORK
        "folder-remote" => 51,              // SIID_SERVERSHARE
        "text-x-generic" => 0,              // SIID_DOCNOASSOC
        _ => return None,
    })
}

/// The first of `names` (freedesktop icon names) Windows has a stock icon for.
pub(crate) fn named_icon_rgba(names: &[&str]) -> Option<(u32, Vec<u8>)> {
    names.iter().filter_map(|name| stock_id(name)).find_map(|id| {
        let mut info = StockIconInfo {
            size: std::mem::size_of::<StockIconInfo>() as u32,
            icon: std::ptr::null_mut(),
            system_index: 0,
            icon_index: 0,
            path: [0; 260],
        };
        // SHGSI_ICON (large): one caller-owned icon.
        if unsafe { SHGetStockIconInfo(id, 0x100, &mut info) } != 0 || info.icon.is_null() {
            return None;
        }
        let rgba = icon_rgba(info.icon);
        unsafe {
            DestroyIcon(info.icon);
        }
        rgba.map(|rgba| (SIDE as u32, rgba))
    })
}
