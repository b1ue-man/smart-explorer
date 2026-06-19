//! Shared protocol and local agent filesystem operations.
//!
//! This module is included by both the app-side transport and the small agent
//! binary, so the wire frames, framing, and server-side local operations stay in
//! one place.
#![allow(dead_code, unused_imports)]

mod codec;
#[cfg(test)]
mod codec_tests;
mod core_oslocked;
mod hash;
mod search;
mod server;
mod session;
mod transfer;
mod types;

pub use codec::{read_frame, write_frame};
pub use core_oslocked::{is_pseudo_dir, list_local, stat_local, walk_local, WalkCounter};
pub use server::serve;
pub use types::{Frame, SearchSpec, WireMeta, WireNode, CHUNK, PROTO_VERSION};
