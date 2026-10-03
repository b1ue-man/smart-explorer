//! Windows host figures and the visible Smart-Explorer host trash.
use crate::analytics::VolumeUsage;
use std::os::windows::ffi::OsStrExt;
use std::{io, path::Path};

pub(crate) fn volume_usage(path: &Path) -> io::Result<VolumeUsage> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            &mut total,
            &mut free,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(VolumeUsage {
        total_bytes: total,
        free_bytes: free,
    })
}

pub(crate) fn recycle(
    root: &Path,
    path: &Path,
    expected: &crate::vfs::RecycleExpectation,
) -> io::Result<crate::vfs::RecycleOutcome> {
    super::checked_recycle::with_regular_child(root, path, |root, _, parent, _, file| {
        crate::host_trash::recycle_selected(root, parent, file, expected)
    })
}
pub(crate) fn host_recycle_available() -> bool {
    crate::host_trash::available()
}

pub(crate) fn host_permission_note(denied: u64) -> Option<String> {
    (denied > 0).then(|| "Einige Bereiche sind auf dem Host nicht lesbar. Eine lokale Analyse mit dort ausdrücklich erlaubten erhöhten Leserechten kann mehr erfassen; Fernanfragen erteilen diese Rechte nicht.".into())
}
