#[path = "core/cloud.rs"]
mod cloud;

mod os {
    #[cfg(target_os = "linux")]
    #[path = "linux_os.rs"]
    mod platform;
    #[cfg(windows)]
    #[path = "windows.rs"]
    mod platform;
    pub mod shared;

    pub use platform::open_url;
}

pub use cloud::*;
