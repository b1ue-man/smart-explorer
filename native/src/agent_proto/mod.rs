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
#[path = "os/shared/fs.rs"]
mod fs;
#[path = "os/shared/hash.rs"]
mod hash;
#[path = "os/shared/search.rs"]
mod search;
#[path = "core/server.rs"]
mod server;
#[path = "core/session.rs"]
mod session;
#[path = "os/shared/transfer.rs"]
mod transfer;
#[path = "core/types.rs"]
mod types;

pub use codec::{read_frame, write_frame};
pub use fs::{is_pseudo_dir, list_local, stat_local, walk_local, WalkCounter};
pub use server::serve;
pub use types::{Frame, SearchSpec, WireMeta, WireNode, CHUNK, PROTO_VERSION};
