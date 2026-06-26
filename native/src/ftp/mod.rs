#[path = "core/ftp.rs"]
mod core_impl;

pub use core_impl::backend_from_url;
