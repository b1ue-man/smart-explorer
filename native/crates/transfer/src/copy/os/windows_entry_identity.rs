//! Volumes whose file numbers follow the directory entry (U13, 2026-11-27).
//!
//! FAT, FAT32 and exFAT derive the number `GetFileInformationByHandle` and
//! `FILE_ID_INFO` report from the place of the file's directory entry
//! (Microsoft Learn, remarks of `BY_HANDLE_FILE_INFORMATION` and
//! `FILE_ID_INFO`): a rename that moves the entry (a longer name needs more
//! entry slots) gives the same file another number, and a deleted file's
//! slot gives a later file the old one. On such a volume the number proves
//! one file only between two live handles (`same_file_object`), so the
//! snapshot of a source (`move_guard::SourceSnapshot`) carries no number
//! there: snapshots compare the volume, the length and the modification
//! time, as robocopy /MOV and `MoveFileEx(MOVEFILE_COPY_ALLOWED)` judge a
//! moved file. Every check of the object itself keeps its live handles.
//!
//! A volume answers once per volume serial number. Two volumes sharing a
//! serial (a cloned disk) can at worst get the other's answer: an NTFS
//! volume then compares snapshots without the number (its live handle
//! checks stay), a FAT volume refuses the move as changed (nothing is lost).
use std::collections::BTreeMap;
use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::sync::{Mutex, MutexGuard};

use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

use super::FileIdentity;

/// FILE_SUPPORTS_OPEN_BY_FILE_ID (winnt.h; windows-sys keeps it in
/// `Win32_System_SystemServices`, a feature this workspace does not enable).
const OPEN_BY_FILE_ID: u32 = 0x0100_0000;
/// `MAX_PATH + 1` UTF-16 units, the documented filesystem-name buffer size.
const NAME_UNITS: usize = 261;

static KNOWN: Mutex<BTreeMap<u32, bool>> = Mutex::new(BTreeMap::new());

fn known() -> MutexGuard<'static, BTreeMap<u32, bool>> {
    KNOWN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a volume with this filesystem name and these filesystem flags
/// numbers its files by directory entry: the FAT family by name, and any
/// filesystem that cannot open a file by its number (the number then names
/// no lasting object either).
pub(super) fn entry_bound(fs_name: &str, flags: u32) -> bool {
    let fat = ["FAT", "FAT12", "FAT16", "FAT32", "exFAT"]
        .iter()
        .any(|name| fs_name.eq_ignore_ascii_case(name));
    fat || flags & OPEN_BY_FILE_ID == 0
}

/// The identity a snapshot of `file` keeps: volume serial and file number,
/// the number left out (0) where it follows the directory entry. A volume
/// that does not answer keeps the number (the earlier comparison).
pub(super) fn snapshot_identity(file: &File, identity: FileIdentity) -> FileIdentity {
    let (serial, index) = identity;
    if follows_entry(file, serial) {
        (serial, 0)
    } else {
        (serial, index)
    }
}

fn follows_entry(file: &File, serial: u32) -> bool {
    if serial != 0 {
        if let Some(answer) = known().get(&serial).copied() {
            return answer;
        }
    }
    let Some(answer) = ask(file) else {
        return false;
    };
    if serial != 0 {
        known().insert(serial, answer);
    }
    answer
}

fn ask(file: &File) -> Option<bool> {
    let mut flags = 0u32;
    let mut name = [0u16; NAME_UNITS];
    // SAFETY: a live handle; the flags and the complete name buffer stay
    // valid for the call, every other output is null with size 0.
    let answered = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            NAME_UNITS as u32,
        )
    };
    if answered == 0 {
        return None;
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(NAME_UNITS);
    Some(entry_bound(
        &String::from_utf16_lossy(&name[..length]),
        flags,
    ))
}

#[cfg(test)]
#[path = "windows_entry_identity_tests.rs"]
mod tests;
