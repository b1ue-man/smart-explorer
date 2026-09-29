#[path = "core/webdav.rs"]
mod core_impl;
#[path = "core/multistatus.rs"]
mod multistatus;
#[path = "core/status.rs"]
mod status;
#[path = "core/stream_put.rs"]
mod stream_put;
#[path = "core/writer.rs"]
mod writer;

#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use core_impl::{WebdavBackend, WebdavConfig};
