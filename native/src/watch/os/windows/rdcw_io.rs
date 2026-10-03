//! Completions, device events and reads of the Windows watch thread (RV1,
//! V4): a finished read becomes changes or `Overflow` and is issued again
//! at once (the kernel keeps buffering in between); a closed handle ends the
//! slot; other failures end the root's watch with a reason.

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ACCESS_DENIED, ERROR_BAD_NETPATH, ERROR_DEVICE_NOT_CONNECTED,
    ERROR_FILE_NOT_FOUND, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NETNAME_DELETED,
    ERROR_NOTIFY_CLEANUP, ERROR_NOTIFY_ENUM_DIR, ERROR_NOT_READY, ERROR_NOT_SUPPORTED,
    ERROR_OPERATION_ABORTED, ERROR_PATH_NOT_FOUND, ERROR_UNEXP_NET_ERR,
};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, ReadDirectoryChangesW};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOTE;
use windows_sys::Win32::System::IO::OVERLAPPED;

use super::super::notify_records::parse;
use super::super::service::emit;
use super::super::types::{Change, UnavailableReason, WatchEvent};
use super::{Slot, State, FILTER};

impl State {
    pub(super) fn device_event(&mut self, key: usize) {
        let Some(slot) = self.slots.get(&key) else {
            return;
        };
        let id = slot.id;
        let closed = slot.handle.with(|_| ()).is_none();
        let current = self.roots.get(&id).and_then(|root| root.slot) == Some(key);
        // An open handle means the removal was refused elsewhere: nothing to
        // do. A closed one: the volume is being removed; look for it again.
        if closed && current {
            self.fail(id, UnavailableReason::DeviceRemoved);
        }
    }

    pub(super) fn completion(&mut self, key: usize, error: u32, bytes: u32) {
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        slot.pending = false;
        if slot.closing {
            self.slots.remove(&key);
            return;
        }
        let id = slot.id;
        let events = match error {
            0 if bytes == 0 => vec![WatchEvent::Overflow],
            0 => {
                let length = usize::try_from(bytes)
                    .unwrap_or(0)
                    .min(slot.buffer.len() * 4);
                // SAFETY: the kernel wrote `length` bytes into the buffer.
                let raw = unsafe {
                    std::slice::from_raw_parts(slot.buffer.as_ptr().cast::<u8>(), length)
                };
                parse(raw)
                    .into_iter()
                    .map(|record| {
                        WatchEvent::Change(Change {
                            rel: record.rel,
                            kind: record.kind,
                            is_dir: None,
                        })
                    })
                    .collect()
            }
            ERROR_NOTIFY_ENUM_DIR => vec![WatchEvent::Overflow],
            ERROR_OPERATION_ABORTED | ERROR_NOTIFY_CLEANUP => {
                // Closed under the read (removal); the device event decides.
                self.slots.remove(&key);
                return;
            }
            other => {
                self.fail(id, reason_of_code(other));
                return;
            }
        };
        emit(id, events);
        let Some(slot) = self.slots.get_mut(&key) else {
            return;
        };
        if let Err(code) = issue(slot) {
            match code {
                ERROR_OPERATION_ABORTED | ERROR_NOTIFY_CLEANUP => {
                    self.slots.remove(&key);
                }
                other => self.fail(id, reason_of_code(other)),
            }
        }
    }
}

/// Starts the next read; `Err(code)` when it could not be started.
pub(super) fn issue(slot: &mut Slot) -> Result<(), u32> {
    let length = u32::try_from(slot.buffer.len() * 4).unwrap_or(u32::MAX);
    // SAFETY: an all-zero OVERLAPPED is valid (no read is in flight here).
    *slot.overlapped = unsafe { std::mem::zeroed() };
    let buffer = slot.buffer.as_mut_ptr().cast();
    let overlapped: *mut OVERLAPPED = &mut *slot.overlapped;
    let started = slot.handle.with(|handle| {
        // SAFETY: buffer and OVERLAPPED live in the slot until the read
        // completes; the handle stays open while the cell lock is held.
        let done = unsafe {
            ReadDirectoryChangesW(
                handle,
                buffer,
                length,
                1,
                FILTER,
                std::ptr::null_mut(),
                overlapped,
                None,
            )
        };
        if done == 0 {
            // SAFETY: reads the calling thread's last error right away.
            Err(unsafe { GetLastError() })
        } else {
            Ok(())
        }
    });
    match started {
        Some(Ok(())) => {
            slot.pending = true;
            Ok(())
        }
        Some(Err(code)) => Err(code),
        None => Err(ERROR_OPERATION_ABORTED),
    }
}

pub(super) fn is_network(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    if text.starts_with(r"\\?\UNC\") || (text.starts_with(r"\\") && !text.starts_with(r"\\?\")) {
        return true;
    }
    let drive = text.trim_start_matches(r"\\?\");
    let Some(letter) = drive.chars().next().filter(|c| c.is_ascii_alphabetic()) else {
        return false;
    };
    if !drive[1..].starts_with(':') {
        return false;
    }
    let root: Vec<u16> = format!("{letter}:\\")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: NUL-terminated drive root.
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

pub(super) fn reason_of_code(code: u32) -> UnavailableReason {
    match code {
        ERROR_INVALID_FUNCTION | ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED => {
            UnavailableReason::Unsupported
        }
        ERROR_FILE_NOT_FOUND
        | ERROR_PATH_NOT_FOUND
        | ERROR_NOT_READY
        | ERROR_BAD_NETPATH
        | ERROR_NETNAME_DELETED
        | ERROR_UNEXP_NET_ERR
        | ERROR_DEVICE_NOT_CONNECTED => UnavailableReason::RootMissing,
        ERROR_ACCESS_DENIED => UnavailableReason::AccessDenied,
        other => UnavailableReason::Failed(
            io::Error::from_raw_os_error(i32::try_from(other).unwrap_or(i32::MAX)).to_string(),
        ),
    }
}

pub(super) fn reason_of_io(error: &io::Error) -> UnavailableReason {
    match error.kind() {
        io::ErrorKind::NotFound => UnavailableReason::RootMissing,
        io::ErrorKind::PermissionDenied => UnavailableReason::AccessDenied,
        _ => match error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
        {
            Some(code) => reason_of_code(code),
            None => UnavailableReason::Failed(error.to_string()),
        },
    }
}
