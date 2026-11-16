//! The stage names of the app's file systems (one definition, included from
//! the app): the shared protocol code names and recognises its range stages
//! with them.
#[path = "../../native/crates/storage/src/vfs/core/staging_names.rs"]
mod staging_names;

pub(crate) use staging_names::{fit_stage_name, is_unique_stage};

// The background marker of a request (E4d): the shared protocol code reads it
// to mark a request id (`agent_proto/core/request_origin.rs`). The agent's
// own threads never set it, so its requests stay unmarked.
#[path = "../../native/crates/storage/src/vfs/core/background.rs"]
mod background;

#[allow(unused_imports)]
pub(crate) use background::{in_background, BackgroundScope};
