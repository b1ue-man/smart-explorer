#[path = "core/webdav.rs"]
mod core_impl;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/listing_body.rs"]
mod listing_body;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/multistatus.rs"]
mod multistatus;
#[path = "core/stage_move.rs"]
mod stage_move;
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
