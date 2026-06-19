//! Shared protocol and local agent filesystem operations.
//!
//! This module is included by both the app-side transport and the small agent
//! binary, so the wire frames, framing, and server-side local operations stay in
//! one place.
#![allow(dead_code, unused_imports)]

#[path = "core/codec.rs"]
mod codec;
#[cfg(test)]
#[path = "core/codec_tests.rs"]
mod codec_tests;
#[path = "core_oslocked/fs.rs"]
mod core_oslocked;
#[path = "core_oslocked/hash.rs"]
mod hash;
#[path = "core_oslocked/search.rs"]
mod search;
#[path = "core_oslocked/server.rs"]
mod server;
#[path = "core_oslocked/session.rs"]
mod session;
#[path = "core_oslocked/transfer.rs"]
mod transfer;
#[path = "core/types.rs"]
mod types;

pub use codec::{read_frame, write_frame};
pub use core_oslocked::{is_pseudo_dir, list_local, stat_local, walk_local, WalkCounter};
pub use server::serve;
pub use types::{Frame, SearchSpec, WireMeta, WireNode, CHUNK, PROTO_VERSION};
