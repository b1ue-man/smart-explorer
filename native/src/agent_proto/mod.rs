//! Shared protocol and local agent filesystem operations.
//!
//! This module is included by both the app-side transport and the small agent
//! binary, so the wire frames, framing, and server-side local operations stay in
//! one place.
#![allow(dead_code, unused_imports)]

#[path = "os/shared/batch_get.rs"]
mod batch_get;
#[path = "core/batch_limits.rs"]
mod batch_limits;
#[path = "os/shared/batch_put.rs"]
mod batch_put;
#[path = "core/codec.rs"]
mod codec;
#[cfg(test)]
#[path = "core/codec_tests.rs"]
mod codec_tests;
#[path = "core/credit.rs"]
mod credit;
#[path = "os/shared/ext_ops.rs"]
mod ext_ops;
#[path = "core/extension_parts.rs"]
mod extension_parts;
#[path = "core/features.rs"]
mod features;
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
#[path = "core/ops_types.rs"]
mod ops_types;
#[path = "os/shared/promotion.rs"]
mod promotion;
#[path = "os/shared/put_tree.rs"]
mod put_tree;
#[path = "core/relative_path.rs"]
mod relative_path;
#[cfg(test)]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/sandbox.rs"]
mod sandbox;
#[path = "os/shared/search.rs"]
mod search;
#[path = "core/server.rs"]
mod server;
#[path = "core/server_session.rs"]
mod server_session;
#[path = "core/session.rs"]
mod session;
#[path = "os/shared/stage_ops.rs"]
mod stage_ops;
#[path = "os/shared/transfer.rs"]
mod transfer;
#[cfg(test)]
#[path = "core/transfer_engine_task_bounds_tests.rs"]
mod transfer_engine_task_bounds_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;
#[path = "core/transport_error.rs"]
mod transport_error;
#[path = "core/types.rs"]
mod types;
#[path = "os/shared/write_new.rs"]
mod write_new;

pub use batch_limits::{
    check_get_batch, check_put_batch, clip_text, get_item_len, numbered_name, put_entry_len,
    split_batch, BATCH_HEADER_MAX, BATCH_MAX_BYTES, BATCH_MAX_FILES, BATCH_UNKNOWN_MARKER,
    ITEM_PATH_MAX, ITEM_TEXT_MAX,
};
pub use codec::{read_frame, read_frame_with_tree_budget, write_frame};
pub use node_codec::{TreeDecodeBudget, TREE_BUDGET_ERROR};
pub use credit::{
    busy_message, charged, credit_cost, parse_busy, window_target, RecvWindow, SendCredit,
    StreamCount, CREDIT_CONNECTION_BUDGET, CREDIT_INITIAL, CREDIT_REQUEST_LIMIT, CREDIT_WINDOW_MAX,
};
pub use features::{
    server_version, server_version_with, service_slots, ServerFeatures, LABEL_BATCH, LABEL_CREDIT,
    LABEL_STAGE,
};
pub use fs::{is_pseudo_dir, list_local, stat_local, walk_local, WalkCounter};
pub(crate) use extension_parts::{append_duplicate_part, emit_duplicate_parts, emit_listing_parts};
pub use ops_types::{digest, omission, query, UNSUPPORTED_EXTENSION};
pub(crate) use promotion::validate_destination_root;
pub(crate) use put_tree::{BufferedTree, BufferedTreeReceiver, TreeManifestValidator};
pub use relative_path::ValidatedRelativePath;
#[cfg(target_os = "linux")]
pub use sandbox::restrict_filesystem;
pub use server::serve;
pub(crate) use server_session::{transfer_channel, RequestContext, ServerSession};
pub(crate) use session::{Inbound, Sink};
pub(crate) use transfer::{
    collect_local_tree, finish_local_tree_file, open_local_tree_file, LocalTreeEntry,
};
pub(crate) use transport_error::{parse_transport_error, transport_error_message};
pub use types::{
    has_link_aware_hash, BatchEntry, BatchItem, Frame, SearchSpec, WireMeta, WireNode, CHUNK,
    HASH_WALK_LINK_BOUNDARY, HASH_WALK_SERVER_VERSION, PROTO_VERSION, TRANSFER_FRAME_BACKLOG,
};
pub use types::{
    WireChange, WireDuplicateGroup, WireDuplicateItem, WireDuplicateSummary, WireOmission,
    WireReclaimProgress, WireTargetLimits,
};
