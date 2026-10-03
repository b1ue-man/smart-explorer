//! Linux host figures and the freedesktop trash.
use crate::analytics::VolumeUsage;
use std::os::unix::ffi::OsStrExt;
use std::{ffi::CString, io, path::Path};

pub(crate) fn volume_usage(path: &Path) -> io::Result<VolumeUsage> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL im Volumenpfad"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let block = stat.f_frsize as u64;
    Ok(VolumeUsage {
        total_bytes: (stat.f_blocks as u64).saturating_mul(block),
        free_bytes: (stat.f_bfree as u64).saturating_mul(block),
    })
}

pub(crate) fn recycle(
    root: &Path,
    path: &Path,
    expected: &crate::vfs::RecycleExpectation,
) -> io::Result<crate::vfs::RecycleOutcome> {
    super::checked_recycle::recycle(root, path, expected, publish_trash)
}
pub(crate) fn host_recycle_available() -> bool {
    true
}
fn publish_trash(
    captured: &mut crate::local_access::QuarantinedChild,
    original: &Path,
) -> io::Result<()> {
    super::linux_trash::publish(captured, original)
}

pub(crate) fn host_permission_note(_: u64) -> Option<String> {
    None
}
