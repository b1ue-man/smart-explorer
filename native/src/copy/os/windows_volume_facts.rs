//! What the process remembers of a volume (A7 follow-up, 2026-10-29): whether
//! it is a network volume and whether it shares clusters between files
//! (ReFS, Dev Drive: block cloning). Both belong to the volume, so they are
//! asked once per volume serial number instead of once per file; the serial
//! comes with the `GetFileInformationByHandle` answer the copy has anyway.
//!
//! The answers only choose between two correct copies (the handle copy of
//! `windows_fast.rs` or `CopyFile2`). Two volumes can share one serial (a
//! cloned disk), and then at worst the slower of the two copies is chosen.
//! Nothing that decides whether a by-handle rename is safe may rest on this
//! cache: `windows_held_rename.rs` asks its own stage handle.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use windows_sys::Win32::Storage::FileSystem::{
    GetVolumeInformationByHandleW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

/// FILE_SUPPORTS_BLOCK_REFCOUNTING in the file system flags: the volume
/// shares logical clusters between files, which is what block cloning needs
/// (Microsoft Learn `GetVolumeInformationByHandleW`, value 0x08000000).
const BLOCK_REFCOUNTING: u32 = 0x0800_0000;

#[derive(Clone, Copy)]
pub(super) struct Facts {
    /// A network volume: a mapped drive keeps `CopyFile2`.
    pub(super) remote: bool,
    /// The volume can clone blocks between its files.
    pub(super) block_clone: bool,
}

static KNOWN: Mutex<BTreeMap<u32, Facts>> = Mutex::new(BTreeMap::new());

thread_local! {
    /// The folder last asked about and its volume serial number: the stages
    /// of one packet share their folder.
    static LAST_FOLDER: RefCell<Option<(PathBuf, u32)>> = const { RefCell::new(None) };
}

fn known() -> MutexGuard<'static, BTreeMap<u32, Facts>> {
    KNOWN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The facts of the volume `file` is on, whose serial number is `serial`
/// (0 = unknown, never cached). A miss asks the handle; `None` when the
/// network question cannot be answered (the caller then keeps `CopyFile2`).
/// A volume that does not answer the cloning question counts as not cloning
/// and is asked again next time.
pub(super) fn of(file: &File, serial: u32) -> Option<Facts> {
    if serial != 0 {
        let hit = known().get(&serial).copied();
        if hit.is_some() {
            return hit;
        }
    }
    let remote = super::probe::remote_volume(file)?;
    // A network volume answers no volume question and keeps `CopyFile2`.
    let cloning = if remote {
        Some(false)
    } else {
        block_cloning(file)
    };
    let facts = Facts {
        remote,
        block_clone: cloning.unwrap_or(false),
    };
    if serial != 0 && cloning.is_some() {
        known().insert(serial, facts);
    }
    Some(facts)
}

fn block_cloning(file: &File) -> Option<bool> {
    let mut flags = 0u32;
    // SAFETY: a live handle; only the file system flags are requested, every
    // other output and buffer is null with size 0 (all optional).
    let answered = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    (answered != 0).then_some(flags & BLOCK_REFCOUNTING != 0)
}

/// Whether the folder `stage` will be made in is on the volume with serial
/// number `serial`, or cannot be told (then `CopyFile2` stays, which clones
/// where it can). Cloning never crosses volumes. Only a performance choice,
/// so the one folder kept per thread may be stale without harm.
pub(super) fn stage_may_share(stage: &Path, serial: u32) -> bool {
    match stage.parent().and_then(folder_volume) {
        Some(volume) => volume == 0 || serial == 0 || volume == serial,
        None => true,
    }
}

fn folder_volume(folder: &Path) -> Option<u32> {
    let remembered = LAST_FOLDER.with(|cell| {
        cell.borrow()
            .as_ref()
            .filter(|(last, _)| last.as_path() == folder)
            .map(|(_, volume)| *volume)
    });
    if remembered.is_some() {
        return remembered;
    }
    let directory = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(crate::local_access::normalize_scan_root(folder))
        .ok()?;
    let volume = super::information(&directory).ok()?.volume_serial_number;
    LAST_FOLDER.with(|cell| *cell.borrow_mut() = Some((folder.to_path_buf(), volume)));
    Some(volume)
}
