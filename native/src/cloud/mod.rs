#[path = "core/cloud.rs"]
mod core_impl;
#[path = "core/token_persistence.rs"]
mod token_persistence;

mod os {
    #[path = "opener.rs"]
    mod opener;
    #[cfg(target_os = "android")]
    #[path = "android.rs"]
    mod platform;
    #[cfg(target_os = "linux")]
    #[path = "linux_os.rs"]
    mod platform;
    #[cfg(windows)]
    #[path = "windows.rs"]
    mod platform;
    pub mod shared;

    pub use opener::set_url_opener;

    /// Opens `url` through the host opener when one is registered, otherwise
    /// through the platform adapter.
    pub fn open_url(url: &str) -> Result<(), String> {
        if opener::open_with_host(url) {
            return Ok(());
        }
        platform::open_url(url)
    }
}

pub use core_impl::*;
pub use os::set_url_opener;
