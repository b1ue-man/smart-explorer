#[path = "core/auth_state.rs"]
mod auth_state;
#[path = "core/cloud.rs"]
mod core_impl;
#[cfg(test)]
#[path = "core/token_persistence.rs"]
mod token_persistence;
pub use auth_state::{AuthError, AuthState, AuthStatus, ProviderFailure};

mod os {
    pub(crate) mod coordinator;
    pub(crate) mod oauth_flow;
    pub(crate) mod oauth_http;
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
    pub(crate) mod session_store;
    pub mod shared;

    pub use opener::set_url_opener;

    /// The platform adapter alone, for a host opener that adds a step of its
    /// own (the terminal prints the address before it opens the browser).
    pub fn open_platform_url(url: &str) -> Result<(), String> {
        platform::open_url(url)
    }

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
pub use os::{open_platform_url, set_url_opener};

#[cfg(test)]
pub(crate) use os::session_store::{
    test_reset as test_reset_session, test_restore as test_restore_session,
    test_snapshot as test_session_snapshot, TestSessionSnapshot,
};
