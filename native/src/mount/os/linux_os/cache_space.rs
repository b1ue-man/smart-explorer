//! Free space of the drive cache under Linux and Android (DRV-M2): `statvfs`
//! in place of `GetDiskFreeSpaceExW`. Like the Windows probe it reports the
//! space this unprivileged process may still use (`f_bavail`), not the
//! space reserved for root.
use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub(crate) struct CacheDiskSpace {
    directory: CString,
}

impl CacheDiskSpace {
    pub(crate) fn new(directory: &Path) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache disk path is not absolute",
            ));
        }
        let directory = CString::new(directory.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "cache disk path contains NUL")
        })?;
        Ok(Self { directory })
    }
}

impl crate::mount::CacheSpaceProbe for CacheDiskSpace {
    fn available_bytes(&self) -> io::Result<u64> {
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(self.directory.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        Ok(widen(stat.f_bavail).saturating_mul(widen(stat.f_frsize)))
    }
}

/// The libc field types differ between glibc and bionic; every value fits.
fn widen<T: TryInto<u64>>(value: T) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}
