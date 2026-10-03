//! `AgentBackend` - a `vfs::Backend` that drives a remote `se-agent` over the
//! multiplexed, versioned framed stdio stream.
//!
//! A channel carries many operations, tagged by `req_id`: a writer thread
//! serializes outgoing frames (control frames ahead of upload data) and a
//! reader thread routes incoming frames to the waiting operation, bounded by
//! per-request credit on servers that announce `credit-v1`. An SSH-deployed
//! agent spreads requests over a pool of exec channels.

#[path = "core/agent_error.rs"]
mod agent_error;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/analysis.rs"]
mod analysis;
#[path = "core/batch_get.rs"]
mod batch_get;
#[path = "core/batch_put.rs"]
mod batch_put;
#[path = "core/deploy.rs"]
mod deploy;
#[path = "core/engine_ops.rs"]
mod engine_ops;
#[path = "core/ext_wire.rs"]
pub(crate) mod ext_wire;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/lanes.rs"]
mod lanes;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/mux.rs"]
mod mux;
#[path = "core/pool.rs"]
mod pool;
#[path = "core/route.rs"]
mod route;
#[path = "core/search.rs"]
mod search;
#[path = "core/stream.rs"]
mod stream;
#[path = "core/transfer.rs"]
mod transfer;
#[path = "core/transport.rs"]
mod transport;
#[path = "core/walk.rs"]
mod walk;

pub use backend::AgentBackend;
#[allow(unused_imports)]
pub use deploy::{artifact_for, deploy_over_sftp, remove_from_sftp, AgentArtifact};
pub(crate) use metadata::vfs_to_wire;

#[cfg(test)]
#[path = "core/error_tests.rs"]
mod error_tests;
#[cfg(test)]
#[path = "core/heartbeat_tests.rs"]
mod heartbeat_tests;
#[cfg(test)]
#[path = "core/remote_drive_task_deploy_tests.rs"]
mod remote_drive_task_deploy_tests;
#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_service_tests.rs"]
mod transfer_engine_task_service_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;
