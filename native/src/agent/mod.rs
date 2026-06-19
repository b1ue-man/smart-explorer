//! `AgentBackend` - a `vfs::Backend` that drives a remote `se-agent` over the
//! multiplexed, streaming protocol-v2 framed stdio stream.
//!
//! One channel carries every operation, tagged by `req_id`: a writer thread
//! serializes outgoing frames and a reader thread routes incoming frames to the
//! waiting operation.

mod backend;
mod deploy;
mod metadata;
mod mux;
mod stream;
mod transfer;

pub use backend::AgentBackend;
#[allow(unused_imports)]
pub use deploy::{artifact_for, deploy_over_sftp, remove_from_sftp, AgentArtifact};

#[cfg(test)]
mod tests;
