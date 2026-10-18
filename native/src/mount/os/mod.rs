#[cfg(windows)]
pub(crate) mod windows;

/// The drive-host flow and the change delivery every filesystem frontend
/// shares (DRV-M2): the Windows host and the Linux FUSE host (DRV-M4) run
/// them; the tests run on every platform.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) mod shared {
    pub(crate) mod change_pump;
    pub(crate) mod host_platform;
    pub(crate) mod host_session;
}

/// Linux and Android: the cache lease and free space (DRV-M2), the FUSE
/// host (DRV-M4, Linux only) and, from DRV-M8, the documents provider.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) mod linux_os;
