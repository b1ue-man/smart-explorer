#[path = "core/connection.rs"]
mod connection;
#[path = "core/ftp.rs"]
mod core_impl;
#[path = "core/io_adapters.rs"]
mod io_adapters;
#[path = "core/pool.rs"]
mod pool;
#[path = "core/resolver.rs"]
mod resolver;
#[path = "core/staging.rs"]
mod staging;
#[path = "core/streams.rs"]
mod streams;
#[path = "core/writer.rs"]
mod writer;

#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use core_impl::backend_from_url;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/metadata.rs"]
mod metadata;
