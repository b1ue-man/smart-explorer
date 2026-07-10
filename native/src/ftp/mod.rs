#[path = "core/ftp.rs"]
mod core_impl;
#[path = "core/io_adapters.rs"]
mod io_adapters;

pub use core_impl::backend_from_url;
