//! Linux metadata, durability and copies operate on the selected live handles.
use super::{CapacitySnapshot, CapacitySource, TargetLimits, VolumeIdentity};
use crate::local_access::ConfinedDirectory;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        io::AsRawFd,
    },
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn qualify(directory: &ConfinedDirectory) -> io::Result<()> {
    if !directory.has_mount_identity() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local confinement requires a verified mount identity",
        ));
    }
    let mut facts = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(directory.file().as_raw_fd(), facts.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let kind = unsafe { facts.assume_init() }.f_type as u64 & 0xffff_ffff;
    // The kernel filesystems below provide descriptor-relative operations and
    // atomic RENAME_NOREPLACE on kernels new enough to report STATX_MNT_ID.
    // Arbitrary FUSE/network implementations keep the original provider path;
    // no unsafe check/rename fallback may turn them into an Enforced adapter.
    let writable = matches!(
        kind,
        0xef53
            | 0x9123_683e
            | 0x0102_1994
            | 0x5846_5342
            | 0x794c_7630
            | 0xf2f5_2010
            | 0x4d44
            | 0x2011_bab0
            | 0x7366_746e
    );
    let read_only = matches!(kind, 0x9660 | 0x1501_3346 | 0x7371_7368 | 0xe0f5_e1e2);
    if !writable && !read_only {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem keeps its existing unverified local/provider contract",
        ));
    }
    directory.verify_reopen_support()
}
fn anchor(directory: &ConfinedDirectory) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.file().as_raw_fd()))
}
pub(super) fn capacity(
    directory: &ConfinedDirectory,
    root: &str,
    identity: String,
) -> io::Result<CapacitySnapshot> {
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::fstatvfs(directory.file().as_raw_fd(), value.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    let unit = wide(value.f_frsize);
    if unit == 0 {
        return Err(overflow());
    }
    let total = wide(value.f_blocks)
        .checked_mul(unit)
        .ok_or_else(overflow)?;
    let free = wide(value.f_bfree).checked_mul(unit).ok_or_else(overflow)?;
    let available = wide(value.f_bavail)
        .checked_mul(unit)
        .ok_or_else(overflow)?;
    let mut result = CapacitySnapshot::known(
        identity,
        root,
        CapacitySource::Filesystem,
        total,
        total.saturating_sub(free),
        available,
    );
    result.read_only = value.f_flag & libc::ST_RDONLY as libc::c_ulong != 0;
    Ok(result)
}
pub(super) fn limits(directory: &ConfinedDirectory) -> io::Result<TargetLimits> {
    super::local_platform::filesystem_profile(&anchor(directory)).map(|profile| profile.limits)
}
pub(super) fn volume(directory: &ConfinedDirectory) -> io::Result<Option<VolumeIdentity>> {
    super::local_platform::volume_identity(&anchor(directory))
}
pub(super) fn set_mode(file: &File, mode: u32) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}
pub(super) fn mode(file: &File) -> io::Result<Option<u32>> {
    Ok(Some(file.metadata()?.mode() & 0o7777))
}
pub(super) fn sync_filesystem(directory: &ConfinedDirectory) -> io::Result<bool> {
    let profile = super::local_platform::filesystem_profile(&anchor(directory))?;
    if profile.flush == super::fs_profile::FlushModel::PerFileOnly {
        return Ok(false);
    }
    if unsafe { libc::syncfs(directory.file().as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}
pub(super) fn confirm_namespace(directory: &ConfinedDirectory) -> io::Result<bool> {
    // Existing qualification resolves only the held FD and verifies its mount.
    super::local_platform::confirm_namespace(&anchor(directory))
}
pub(super) fn copy_permissions(
    source: &File,
    stage: &File,
    destination: Option<&File>,
) -> io::Result<()> {
    let mut bits = source.metadata()?.permissions().mode() & 0o777;
    if let Some(destination) = destination {
        if destination.metadata()?.is_file() {
            bits &= destination.metadata()?.permissions().mode();
        }
    }
    match set_mode(stage, bits) {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied
                    | io::ErrorKind::Unsupported
                    | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        result => result,
    }
}
pub(super) fn copy(
    source: &mut File,
    destination: &mut File,
    cancel: &AtomicBool,
    expected: Option<u64>,
) -> io::Result<u64> {
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "local copy cancelled",
            ));
        }
        let remaining = expected.map_or(8 * 1024 * 1024, |size| {
            size.saturating_sub(copied).min(8 * 1024 * 1024)
        });
        if remaining == 0 {
            return Ok(copied);
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_copy_file_range,
                source.as_raw_fd(),
                std::ptr::null_mut::<i64>(),
                destination.as_raw_fd(),
                std::ptr::null_mut::<i64>(),
                remaining as usize,
                0u32,
            )
        };
        if result == 0 {
            return Ok(copied);
        }
        if result > 0 {
            copied = copied.saturating_add(result as u64);
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if !matches!(
            error.raw_os_error(),
            Some(libc::ENOSYS | libc::EXDEV | libc::EINVAL | libc::EOPNOTSUPP)
        ) {
            return Err(error);
        }
        break;
    }
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "local copy cancelled",
            ));
        }
        let limit = expected.map_or(buffer.len(), |size| {
            size.saturating_sub(copied).min(buffer.len() as u64) as usize
        });
        if limit == 0 {
            return Ok(copied);
        }
        let read = source.read(&mut buffer[..limit])?;
        if read == 0 {
            return Ok(copied);
        }
        destination.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
    }
}
fn wide<T: TryInto<u64>>(value: T) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}
fn overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "filesystem capacity overflow")
}

pub(super) fn stage_write_access() -> bool {
    false
}
pub(super) fn validate_stage(file: &File) -> io::Result<()> {
    if file.metadata()?.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "stage has another hardlink",
        ));
    }
    Ok(())
}
