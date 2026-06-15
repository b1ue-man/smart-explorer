// CF_HDROP clipboard interop: lets us Copy/Cut files in our app such that
// they can be pasted into Windows Explorer, Outlook, Word, etc., and vice
// versa (reading CF_HDROP for paste into our app).
//
// Format details:
//   - CF_HDROP (15) carries a DROPFILES struct followed by a double-null-
//     terminated wide-char list of paths.
//   - The registered "Preferred DropEffect" clipboard format carries a
//     DWORD: DROPEFFECT_COPY (1) for copy, DROPEFFECT_MOVE (2) for cut.
//     Explorer reads this to know whether to copy or move on paste.

#![cfg(windows)]

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, POINT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::DROPFILES;

pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;

fn registered_format(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
}

fn preferred_drop_effect_fmt() -> u32 {
    registered_format("Preferred DropEffect")
}

/// Write the given paths to the clipboard as CF_HDROP with the given effect
/// (copy or cut). After this, pasting in Explorer will perform that action.
pub fn write_files(paths: &[String], effect: u32) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    // Build the wide string list: each path null-terminated, list double-null
    let mut wide: Vec<u16> = Vec::with_capacity(paths.iter().map(|p| p.len() + 1).sum());
    for p in paths {
        let normalized = p.replace('/', "\\");
        for c in normalized.encode_utf16() {
            wide.push(c);
        }
        wide.push(0);
    }
    wide.push(0); // double-null terminator

    let header_size = std::mem::size_of::<DROPFILES>();
    let data_size = wide.len() * 2;
    let total_size = header_size + data_size;

    unsafe {
        // Allocate moveable global memory for the clipboard
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, total_size)?;
        let ptr = GlobalLock(hglobal) as *mut u8;
        if ptr.is_null() {
            return Err(windows::core::Error::from_win32());
        }

        // Write DROPFILES header
        let header = ptr as *mut DROPFILES;
        (*header).pFiles = header_size as u32;
        (*header).pt = POINT { x: 0, y: 0 };
        (*header).fNC = false.into();
        (*header).fWide = true.into();

        // Write the wide-string list
        let str_ptr = ptr.add(header_size) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), str_ptr, wide.len());

        let _ = GlobalUnlock(hglobal);

        // Open clipboard, empty, set CF_HDROP + Preferred DropEffect
        OpenClipboard(HWND::default())?;
        let _ = EmptyClipboard();

        SetClipboardData(CF_HDROP.0 as u32, HANDLE(hglobal.0 as *mut _))?;

        // Preferred DropEffect (cut vs copy)
        let de_fmt = preferred_drop_effect_fmt();
        if de_fmt != 0 {
            let de_handle = GlobalAlloc(GMEM_MOVEABLE, 4)?;
            let de_ptr = GlobalLock(de_handle) as *mut u32;
            if !de_ptr.is_null() {
                *de_ptr = effect;
                let _ = GlobalUnlock(de_handle);
                let _ = SetClipboardData(de_fmt, HANDLE(de_handle.0 as *mut _));
            }
        }

        let _ = CloseClipboard();
    }
    Ok(())
}

/// Read the clipboard if it has CF_HDROP. Returns (paths, is_cut).
pub fn read_files() -> Option<(Vec<String>, bool)> {
    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return None;
        }

        let handle = GetClipboardData(CF_HDROP.0 as u32);
        let h = match handle {
            Ok(h) => h,
            Err(_) => {
                let _ = CloseClipboard();
                return None;
            }
        };
        let hglobal: HGLOBAL = HGLOBAL(h.0 as *mut _);
        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return None;
        }

        let header = ptr as *const DROPFILES;
        let offset = (*header).pFiles as usize;
        let wide_flag = (*header).fWide.as_bool();
        let mut paths: Vec<String> = Vec::new();
        if wide_flag {
            // Walk null-terminated wide strings until empty string
            let mut p = ptr.add(offset) as *const u16;
            loop {
                // Find length to next null
                let mut end = p;
                while *end != 0 {
                    end = end.add(1);
                }
                let len = end.offset_from(p) as usize;
                if len == 0 {
                    break;
                }
                let slice = std::slice::from_raw_parts(p, len);
                paths.push(String::from_utf16_lossy(slice));
                p = end.add(1);
            }
        } else {
            // ANSI variant: walk null-terminated bytes
            let mut p = ptr.add(offset);
            loop {
                let mut end = p;
                while *end != 0 {
                    end = end.add(1);
                }
                let len = end.offset_from(p) as usize;
                if len == 0 {
                    break;
                }
                let slice = std::slice::from_raw_parts(p, len);
                paths.push(String::from_utf8_lossy(slice).to_string());
                p = end.add(1);
            }
        }
        let _ = GlobalUnlock(hglobal);

        // Read Preferred DropEffect to know cut vs copy
        let mut is_cut = false;
        let de_fmt = preferred_drop_effect_fmt();
        if de_fmt != 0 {
            if let Ok(de_h) = GetClipboardData(de_fmt) {
                let de_global: HGLOBAL = HGLOBAL(de_h.0 as *mut _);
                let de_ptr = GlobalLock(de_global) as *const u32;
                if !de_ptr.is_null() {
                    is_cut = (*de_ptr) & DROPEFFECT_MOVE != 0;
                    let _ = GlobalUnlock(de_global);
                }
            }
        }

        let _ = CloseClipboard();
        Some((paths, is_cut))
    }
}

