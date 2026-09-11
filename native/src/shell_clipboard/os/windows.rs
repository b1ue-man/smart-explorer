//! Owned CF_HDROP clipboard publication and bounded Windows filename decoding.
#![cfg(windows)]

#[path = "windows/memory.rs"]
mod memory;
#[path = "windows/owner.rs"]
mod owner;
#[path = "windows/read.rs"]
mod read;
#[path = "windows/write.rs"]
mod write;

pub use read::read_files;
pub use write::{write_files, write_files_if_sequence};

use windows::core::{Error, Result, PCWSTR};
use windows::Win32::{
    Foundation::{BOOL, E_INVALIDARG, POINT},
    System::DataExchange::RegisterClipboardFormatW,
    UI::Shell::DROPFILES,
};

pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;

// Bound both allocation and object-count amplification from untrusted input.
const MAX_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;
const MAX_CLIPBOARD_PATHS: usize = 1_000_000;
const HEADER_BYTES: usize = std::mem::size_of::<DROPFILES>();

fn invalid(message: &str) -> Error { Error::new(E_INVALIDARG, message) }

fn preferred_drop_effect_fmt() -> Result<u32> {
    let name: Vec<u16> = "Preferred DropEffect".encode_utf16().chain(Some(0)).collect();
    let format = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    if format == 0 { Err(Error::from_win32()) } else { Ok(format) }
}

fn header(wide: bool) -> Vec<u8> {
    let value = DROPFILES {
        pFiles: HEADER_BYTES as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: BOOL(0),
        fWide: BOOL(i32::from(wide)),
    };
    let mut bytes = vec![0; HEADER_BYTES];
    // windows 0.58 declares DROPFILES packed(1); never borrow its fields.
    unsafe { std::ptr::write_unaligned(bytes.as_mut_ptr().cast::<DROPFILES>(), value); }
    bytes
}
