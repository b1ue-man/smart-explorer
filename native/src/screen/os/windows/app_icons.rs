//! Host-owned HWND/process identity, window HICON and Shell executable icons.
//! No remote path reaches Shell; an AUMID is preferred to executable identity.
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::io;
use windows::core::{w, Interface, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, SIZE};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, IShellItemImageFactory,
    SHCreateItemFromParsingName, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::screen::core::app_icon::{AuthorizedIconEntry, IconReply};
use crate::screen::core::window_list::{AppEntry, WindowList};

#[path = "app_icons_shell.rs"]
mod shell;
use shell::shell_bitmap;

const SIDE: usize = 128;
#[path = "app_icons_path.rs"]
mod path_icon;
pub(crate) use path_icon::{named_icon_rgba, path_icon_rgba};
type RawHandle = *mut c_void;

#[repr(C)]
struct ShellFileInfo {
    icon: RawHandle,
    index: i32,
    attributes: u32,
    display_name: [u16; 260],
    type_name: [u16; 80],
}

#[repr(C)]
struct BitmapInfo {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    image_size: u32,
    x_pixels: i32,
    y_pixels: i32,
    colors_used: u32,
    colors_important: u32,
    color: u32,
}

#[repr(C)]
struct BitmapFacts {
    kind: i32,
    width: i32,
    height: i32,
    stride: i32,
    planes: u16,
    bits: u16,
    data: RawHandle,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetApplicationUserModelId(process: RawHandle, length: *mut u32, id: *mut u16) -> i32;
}
#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetFileInfoW(
        path: *const u16,
        attributes: u32,
        info: *mut ShellFileInfo,
        bytes: u32,
        flags: u32,
    ) -> usize;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SendMessageTimeoutW(
        window: RawHandle,
        message: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
    fn GetClassLongPtrW(window: RawHandle, index: i32) -> usize;
    fn DrawIconEx(
        dc: RawHandle,
        x: i32,
        y: i32,
        icon: RawHandle,
        width: i32,
        height: i32,
        step: u32,
        brush: RawHandle,
        flags: u32,
    ) -> i32;
    fn DestroyIcon(icon: RawHandle) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: RawHandle) -> RawHandle;
    fn CreateDIBSection(
        dc: RawHandle,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut RawHandle,
        section: RawHandle,
        offset: u32,
    ) -> RawHandle;
    fn SelectObject(dc: RawHandle, object: RawHandle) -> RawHandle;
    fn DeleteObject(object: RawHandle) -> i32;
    fn DeleteDC(dc: RawHandle) -> i32;
    fn GdiFlush() -> i32;
    fn GetObjectW(object: RawHandle, size: i32, data: *mut c_void) -> i32;
    fn GetDIBits(
        dc: RawHandle,
        bitmap: RawHandle,
        first: u32,
        rows: u32,
        bits: RawHandle,
        info: *mut BitmapInfo,
        usage: u32,
    ) -> i32;
}

struct Identity {
    id: String,
    path: String,
    pid: u32,
    packaged: bool,
}

fn identity(hwnd: HWND) -> Option<Identity> {
    let mut pid = 0;
    // SAFETY: PID points to a live value. The OS validates HWND/process handles.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()? };
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let query = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let mut aumid = [0u16; 1024];
    let mut id_length = aumid.len() as u32;
    let app_result =
        unsafe { GetApplicationUserModelId(process.0, &mut id_length, aumid.as_mut_ptr()) };
    unsafe {
        let _ = CloseHandle(process);
    }
    let path = query
        .ok()
        .map(|_| String::from_utf16_lossy(&buffer[..length as usize]))?;
    let packaged = app_result == 0 && id_length > 1 && id_length <= aumid.len() as u32;
    let id = if packaged {
        format!(
            "windows-aumid:{}",
            String::from_utf16_lossy(&aumid[..id_length as usize - 1])
        )
    } else {
        format!(
            "windows-exe:{:x}",
            Sha256::digest(path.to_lowercase().as_bytes())
        )
    };
    Some(Identity {
        id,
        path,
        pid,
        packaged,
    })
}

/// Same current process identity used by icons and generation-bound remote windows.
pub(crate) fn window_identity(native: u64) -> io::Result<Option<String>> {
    let handle = usize::try_from(native)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "window handle out of range"))?;
    Ok(identity(HWND(handle as *mut _)).map(|identity| identity.id))
}

fn window_icon(hwnd: HWND) -> Option<RawHandle> {
    let mut result = 0usize;
    // WM_GETICON/ICON_BIG; fail promptly for hung apps and never broadcast.
    let sent =
        unsafe { SendMessageTimeoutW(hwnd.0, 0x007f, 1, 0, 0x0002 | 0x0020, 50, &mut result) };
    if sent != 0 && result != 0 {
        return Some(result as RawHandle);
    }
    // GCLP_HICON is borrowed from the window class, never destroyed by us.
    let class = unsafe { GetClassLongPtrW(hwnd.0, -14) };
    (class != 0).then_some(class as RawHandle)
}

fn shell_icon(identity: &Identity) -> Option<RawHandle> {
    // A generic process host is not a proven app icon. Packaged apps use their window icon.
    let file = identity
        .path
        .rsplit(['\\', '/'])
        .next()?
        .to_ascii_lowercase();
    if identity.packaged || matches!(file.as_str(), "applicationframehost.exe" | "wwahost.exe") {
        return None;
    }
    let path: Vec<u16> = identity.path.encode_utf16().chain(Some(0)).collect();
    let mut info = ShellFileInfo {
        icon: std::ptr::null_mut(),
        index: 0,
        attributes: 0,
        display_name: [0; 260],
        type_name: [0; 80],
    };
    // SHGFI_ICON|SHGFI_LARGEICON returns one caller-owned icon, not an image-list handle.
    let result = unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<ShellFileInfo>() as u32,
            0x100,
        )
    };
    (result != 0 && !info.icon.is_null()).then_some(info.icon)
}

fn render_on(icon: RawHandle, background: u8) -> Option<Vec<u8>> {
    // Only this fixed-size top-down DIB is allocated; no untrusted icon dimensions are used.
    let info = BitmapInfo {
        size: 40,
        width: SIDE as i32,
        height: -(SIDE as i32),
        planes: 1,
        bit_count: 32,
        compression: 0,
        image_size: 0,
        x_pixels: 0,
        y_pixels: 0,
        colors_used: 0,
        colors_important: 0,
        color: 0,
    };
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, 0, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        if previous.is_null() || previous as isize == -1 {
            DeleteObject(bitmap);
            DeleteDC(dc);
            return None;
        }
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, SIDE * SIDE * 4);
        pixels.fill(background);
        let drawn = DrawIconEx(
            dc,
            0,
            0,
            icon,
            SIDE as i32,
            SIDE as i32,
            0,
            std::ptr::null_mut(),
            3,
        );
        GdiFlush();
        let copied = (drawn != 0).then(|| pixels.to_vec());
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        copied
    }
}

/// Straight-alpha RGBA (`SIDE`×`SIDE`) of one icon.
fn icon_rgba(icon: RawHandle) -> Option<Vec<u8>> {
    let black = render_on(icon, 0)?;
    let white = render_on(icon, 255)?;
    let mut rgba = Vec::with_capacity(SIDE * SIDE * 4);
    // Two backgrounds recover alpha for both legacy masks and modern alpha icons.
    for (dark, light) in black.as_chunks::<4>().0.iter().zip(white.as_chunks::<4>().0) {
        let transparent = (0..3)
            .map(|channel| light[channel].saturating_sub(dark[channel]))
            .max()
            .unwrap_or(0);
        let alpha = 255u16 - u16::from(transparent);
        let straight = |channel: usize| {
            (u16::from(dark[channel]) * 255 + alpha / 2)
                .checked_div(alpha)
                .map_or(0, |value| value.min(255) as u8)
        };
        rgba.extend_from_slice(&[straight(2), straight(1), straight(0), alpha as u8]);
    }
    Some(rgba)
}

fn raster(icon: RawHandle, app_id: &str) -> Option<IconReply> {
    let rgba = icon_rgba(icon)?;
    let png = super::app_icon_raster::rgba_png(SIDE as u32, SIDE as u32, &rgba).ok()?;
    IconReply::new(app_id, &png).ok()
}

fn resolve(hwnd: HWND, identity: &Identity) -> Option<IconReply> {
    if let Some(icon) = window_icon(hwnd) {
        if let Some(reply) = raster(icon, &identity.id) {
            return Some(reply);
        }
    }
    if identity.packaged {
        return catalog_icon(&identity.id);
    }
    let icon = shell_icon(identity)?;
    let reply = raster(icon, &identity.id);
    unsafe {
        DestroyIcon(icon);
    }
    reply
}

pub(crate) fn attach(list: &mut WindowList) {
    for window in &mut list.windows {
        let Some(hwnd) = window
            .id
            .parse::<usize>()
            .ok()
            .filter(|h| *h != 0)
            .map(|h| HWND(h as RawHandle))
        else {
            continue;
        };
        let Some(identity) = identity(hwnd).filter(|id| Some(id.pid) == window.pid) else {
            window.icon = None;
            continue;
        };
        window.app_id = Some(identity.id.clone());
        window.icon = resolve(hwnd, &identity).map(|reply| reply.icon);
    }
}

pub(crate) fn load(entry: AuthorizedIconEntry<'_>) -> io::Result<Option<IconReply>> {
    let window = match entry {
        AuthorizedIconEntry::Window(window) => window,
        AuthorizedIconEntry::App(app) => {
            return Ok(
                catalog_icon(&app.app_id).filter(|reply| app.icon.as_ref() == Some(&reply.icon))
            )
        }
    };
    let Some(hwnd) = window
        .id
        .parse::<usize>()
        .ok()
        .filter(|h| *h != 0)
        .map(|h| HWND(h as RawHandle))
    else {
        return Ok(None);
    };
    let Some(identity) = identity(hwnd).filter(|identity| {
        Some(identity.pid) == window.pid && window.app_id.as_deref() == Some(identity.id.as_str())
    }) else {
        return Ok(None);
    };
    Ok(resolve(hwnd, &identity).filter(|reply| window.icon.as_ref() == Some(&reply.icon)))
}

fn shell_name(item: &IShellItem) -> Option<String> {
    let name = unsafe { item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()? };
    let value = unsafe { name.to_string().ok() };
    unsafe {
        CoTaskMemFree(Some(name.0 as *const c_void));
    }
    value.filter(|value| value.len() < crate::screen::core::app_icon::MAX_APP_ID_BYTES)
}

/// Re-enumerate AppsFolder and compare identities; a request never becomes a parsing path.
fn catalog_item(app_id: &str) -> Option<IShellItem> {
    let id = app_id.strip_prefix("windows-aumid:")?;
    catalog_items()?.remove(id).flatten()
}

fn catalog_items() -> Option<std::collections::BTreeMap<String, Option<IShellItem>>> {
    let folder: IShellItem =
        unsafe { SHCreateItemFromParsingName(w!("shell:AppsFolder"), None).ok()? };
    let enumerator: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems).ok()? };
    let mut selected = std::collections::BTreeMap::new();
    for _ in 0..8192 {
        let mut items = [None];
        let mut fetched = 0;
        unsafe {
            enumerator.Next(&mut items, Some(&mut fetched)).ok()?;
        }
        if fetched == 0 {
            return Some(selected);
        }
        let item = items[0].take()?;
        let id = shell_name(&item)?;
        selected
            .entry(id)
            .and_modify(|entry| *entry = None)
            .or_insert(Some(item));
    }
    None // Truncation cannot prove a unique catalog identity.
}

/// Used only after the AppHost's installed catalog has bound this ID to a visible entry.
pub(crate) fn catalog_icon(app_id: &str) -> Option<IconReply> {
    if !crate::screen::core::app_icon::valid_app_id(app_id) {
        return None;
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() && initialized.0 != 0x80010106u32 as i32 {
        return None;
    }
    let result = catalog_item(app_id).and_then(|item| shell_bitmap(&item, app_id));
    if initialized.is_ok() {
        unsafe {
            CoUninitialize();
        }
    }
    result
}

pub(crate) fn attach_apps(apps: &mut [AppEntry]) {
    for app in apps.iter_mut() {
        app.icon = None;
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() && initialized.0 != 0x80010106u32 as i32 {
        return;
    }
    if let Some(items) = catalog_items() {
        for app in apps {
            let item = app
                .app_id
                .strip_prefix("windows-aumid:")
                .and_then(|id| items.get(id))
                .and_then(Option::as_ref);
            app.icon = item
                .and_then(|item| shell_bitmap(item, &app.app_id))
                .map(|reply| reply.icon);
        }
    }
    if initialized.is_ok() {
        unsafe {
            CoUninitialize();
        }
    }
}
