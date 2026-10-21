//! The stage names of the app's file systems (one definition, included from
//! the app): the shared protocol code names and recognises its range stages
//! with them.
#[path = "../../native/src/vfs/core/staging_names.rs"]
mod staging_names;

pub(crate) use staging_names::{fit_stage_name, is_unique_stage};
