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
#[cfg(not(windows))]
#[path = "os/linux_os/local_platform.rs"]
mod local_platform;
#[cfg(windows)]
#[path = "os/windows/local_platform.rs"]
mod local_platform;
#[path = "core/node_codec.rs"]
mod node_codec;
#[path = "os/shared/promotion.rs"]
mod promotion;
#[path = "os/shared/put_tree.rs"]
mod put_tree;
#[path = "core/relative_path.rs"]
mod relative_path;
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
pub(crate) use promotion::validate_destination_root;
pub(crate) use put_tree::{BufferedTree, BufferedTreeReceiver, TreeManifestValidator};
pub use relative_path::ValidatedRelativePath;
pub use server::serve;
pub(crate) use transfer::{
    collect_local_tree, finish_local_tree_file, open_local_tree_file, LocalTreeEntry,
};
pub use types::{
    Frame, SearchSpec, WireMeta, WireNode, CHUNK, PROTO_VERSION, TRANSFER_FRAME_BACKLOG,
};
