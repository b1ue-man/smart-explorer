#[cfg(windows)]
pub(crate) mod windows;

/// The drive-host flow and the change delivery every filesystem frontend
/// shares (DRV-M2). Until the Linux FUSE host (DRV-M4) exists, only the
/// Windows host runs them; the tests run on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod shared {
    pub(crate) mod change_pump;
    pub(crate) mod host_platform;
    pub(crate) mod host_session;
}

/// Cache ownership and free space under Linux and Android (DRV-M2), for the
/// FUSE host (DRV-M4) and the documents provider (DRV-M8).
#[cfg(any(target_os = "linux", target_os = "android"))]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod linux_os {
    pub(crate) mod cache_lease;
    pub(crate) mod cache_space;
}
