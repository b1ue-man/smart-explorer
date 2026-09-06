use std::{io, os::windows::ffi::OsStrExt, path::Path};

use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

pub(super) struct CacheDiskSpace {
    directory: Vec<u16>,
}

impl CacheDiskSpace {
    pub(super) fn new(directory: &Path) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "cache disk path is not absolute"));
        }
        let mut encoded = directory.as_os_str().encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "cache disk path contains NUL"));
        }
        encoded.push(0);
        Ok(Self { directory: encoded })
    }
}

impl crate::mount::cache_space::CacheSpaceProbe for CacheDiskSpace {
    fn available_bytes(&self) -> io::Result<u64> {
        let mut available = 0u64;
        if unsafe {
            GetDiskFreeSpaceExW(self.directory.as_ptr(), &mut available,
                std::ptr::null_mut(), std::ptr::null_mut())
        } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(available)
    }
}
