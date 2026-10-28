#[path = "core/chunked_stage.rs"]
mod chunked_stage;
#[path = "core/chunked_upload.rs"]
mod chunked_upload;
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
#[path = "core/put_stages.rs"]
mod put_stages;
#[path = "core/stage_move.rs"]
mod stage_move;
#[path = "core/status.rs"]
mod status;
#[path = "core/stream_put.rs"]
mod stream_put;
#[path = "core/tree_remove.rs"]
mod tree_remove;
#[path = "core/writer.rs"]
mod writer;

#[cfg(test)]
#[path = "core/chunked_stage_tests.rs"]
mod chunked_stage_tests;
#[cfg(test)]
#[path = "core/delete_speed_task_tests.rs"]
mod delete_speed_task_tests;
#[cfg(test)]
#[path = "core/range_tests.rs"]
mod range_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

#[cfg(test)]
#[path = "core/sync_reliability_task_provider_redirect_tests.rs"]
mod sync_reliability_task_provider_redirect_tests;

#[cfg(test)]
#[path = "core/sync_reliability_task_provider_collections_tests.rs"]
mod sync_reliability_task_provider_collections_tests;

pub use core_impl::{WebdavBackend, WebdavConfig};

#[path = "core/capacity.rs"]
mod capacity;

#[path = "core/folder_reservation.rs"]
mod folder_reservation;
