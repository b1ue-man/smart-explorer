//! Query the selected local/UNC volume with caller-specific free space.
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, LocalBackend, VfsResult};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
pub(super) fn query(root: &str) -> VfsResult<CapacitySnapshot> {
    let path = super::local_platform::to_os(root);
    let path = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let (mut available, mut total, mut free) = (0, 0, 0);
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut available, &mut total, &mut free) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(CapacitySnapshot::known(LocalBackend::new(root).state_identity(), root,
        CapacitySource::Filesystem, total, total.saturating_sub(free), available))
}
