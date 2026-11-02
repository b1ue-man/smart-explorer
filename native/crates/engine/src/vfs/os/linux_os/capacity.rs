//! The filesystem containing the requested location, including mounted SMB/UNC equivalents.
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, LocalBackend, VfsResult};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
pub(super) fn query(root: &str) -> VfsResult<CapacitySnapshot> {
    let path = super::local_platform::to_os(root);
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "capacity path contains NUL",
        )
    })?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let unit = widen(stat.f_frsize);
    let total = widen(stat.f_blocks)
        .checked_mul(unit)
        .ok_or_else(overflow)?;
    let free = widen(stat.f_bfree).checked_mul(unit).ok_or_else(overflow)?;
    let available = widen(stat.f_bavail)
        .checked_mul(unit)
        .ok_or_else(overflow)?;
    let mut result = CapacitySnapshot::known(
        LocalBackend::new(root).state_identity(),
        root,
        CapacitySource::Filesystem,
        total,
        total.saturating_sub(free),
        available,
    );
    result.read_only = stat.f_flag & libc::ST_RDONLY as libc::c_ulong != 0;
    Ok(result)
}
fn overflow() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "filesystem capacity overflow",
    )
}
fn widen<T: TryInto<u64>>(value: T) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}
