#[path = "core/connection.rs"]
mod connection;
#[path = "core/ftp.rs"]
mod core_impl;
#[path = "core/io_adapters.rs"]
mod io_adapters;
#[path = "core/resolver.rs"]
mod resolver;
#[path = "core/writer.rs"]
mod writer;

pub use core_impl::backend_from_url;
