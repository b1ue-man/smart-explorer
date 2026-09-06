//! Conservative circuit breaker: an unfinished private attempt selects System32
//! on the next retry. It neither detects every hang nor replays pending writes.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};

use windows_sys::Win32::Storage::FileSystem::{
    SetFileInformationByHandle, FileDispositionInfo, FILE_DISPOSITION_INFO,
    FILE_FLAG_OPEN_REPARSE_POINT,
};

use crate::mount::MountId;
use super::{cache_lease::validate_lock_handle, private_payload::BUNDLED_DOKANY_SHA256};

pub(super) struct RuntimeAttempt {
    file: File,
}

impl RuntimeAttempt {
    /// Called only while this host owns CacheLease for the same mount ID.
    /// Any existing object is a reason to use compatibility mode; never
    /// truncate/delete an unknown or unfinished previous attempt.
    pub(super) fn arm(cache_root: &Path, id: &MountId) -> io::Result<Self> {
        if BUNDLED_DOKANY_SHA256.is_empty() {
            return Err(io::Error::other("private Dokany payload is unavailable"));
        }
        let marker = cache_root.join(id.as_str())
            .join(format!("private-{}.attempt", BUNDLED_DOKANY_SHA256));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            // GENERIC_READ | GENERIC_WRITE | DELETE. Keep this exact marker
            // exclusively owned through completion, including deletion by handle.
            .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&marker)?;
        validate_lock_handle(&file)?;
        file.write_all(BUNDLED_DOKANY_SHA256.as_bytes())?;
        file.sync_all()?;
        Ok(Self { file })
    }

    pub(super) fn complete(self) -> io::Result<()> {
        // Delete the originally created object, not a re-resolved pathname.
        // Drop without this call leaves it intact after failure or a crash.
        let information = FILE_DISPOSITION_INFO { DeleteFile: 1 };
        if unsafe {
            SetFileInformationByHandle(self.file.as_raw_handle() as _, FileDispositionInfo,
                &information as *const _ as _, std::mem::size_of_val(&information) as u32)
        } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
