#[path = "core/cloud.rs"]
mod cloud;

mod os {
    #[cfg(not(windows))]
    #[path = "non_windows.rs"]
    mod platform;
    #[cfg(windows)]
    #[path = "windows.rs"]
    mod platform;
    pub mod shared;

    pub use platform::open_url;
}

pub use cloud::*;
