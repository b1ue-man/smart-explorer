//! Shared protocol and local agent filesystem operations.
//!
//! This module is included by both the app-side transport and the small agent
//! binary, so the wire frames, framing, and server-side local operations stay in
//! one place.
#![allow(dead_code, unused_imports)]

#[cfg(test)]
#[path = "core/abgleich_task_walk_filter_tests.rs"]
mod abgleich_task_walk_filter_tests;
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
#[cfg(test)]
#[path = "core/content_digest_task_tests.rs"]
mod content_digest_task_tests;
#[path = "core/credit.rs"]
mod credit;
#[cfg(test)]
#[path = "core/delete_speed_task_tests.rs"]
mod delete_speed_task_tests;
#[path = "os/shared/ext_ops.rs"]
mod ext_ops;
#[path = "core/extension_parts.rs"]
mod extension_parts;
#[path = "core/features.rs"]
mod features;
#[path = "os/shared/file_digest.rs"]
mod file_digest;
#[path = "os/shared/file_identity.rs"]
mod file_identity;
#[path = "core/folder_key.rs"]
mod folder_key;
#[path = "os/shared/fs.rs"]
mod fs;
#[path = "os/shared/hash.rs"]
mod hash;
#[path = "os/shared/hash_cache.rs"]
pub mod hash_cache;
#[path = "core/key_tally.rs"]
mod key_tally;
#[cfg(not(windows))]
#[path = "os/linux_os/local_platform.rs"]
mod local_platform;
#[cfg(windows)]
#[path = "os/windows/local_platform.rs"]
mod local_platform;
#[path = "os/shared/meta_list.rs"]
mod meta_list;
#[path = "core/node_codec.rs"]
mod node_codec;
#[path = "core/ops_types.rs"]
mod ops_types;
#[path = "os/shared/ordered_walk.rs"]
mod ordered_walk;
#[path = "os/shared/promotion.rs"]
mod promotion;
#[path = "os/shared/put_tree.rs"]
mod put_tree;
#[path = "os/shared/range_journal.rs"]
mod range_journal;
#[cfg(test)]
#[path = "core/range_lz4_tests.rs"]
mod range_lz4_tests;
#[path = "os/shared/range_ops.rs"]
mod range_ops;
#[path = "os/shared/range_space.rs"]
mod range_space;
#[path = "os/shared/range_stage.rs"]
mod range_stage;
#[path = "core/relative_path.rs"]
mod relative_path;
#[cfg(test)]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;
#[path = "os/shared/run_digests.rs"]
mod run_digests;
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
#[path = "core/sha256.rs"]
mod sha256;
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
#[path = "os/shared/tree_remove.rs"]
mod tree_remove;
#[path = "core/types.rs"]
mod types;
#[path = "core/vault_error.rs"]
mod vault_error;
#[path = "core/walk_filter.rs"]
mod walk_filter;
#[path = "os/shared/walk_hashed.rs"]
mod walk_hashed;
#[path = "os/shared/walk_kept_out.rs"]
mod walk_kept_out;
#[cfg(test)]
#[path = "os/shared/walk_kept_out_tests.rs"]
mod walk_kept_out_tests;
#[path = "os/shared/walk_keyed.rs"]
mod walk_keyed;
pub use walk_keyed::{
    local_keyed, local_keyed_reading, Collected as LocalKeyed, WireItem as KeyedWireItem,
};
#[path = "core/walk_pattern.rs"]
mod walk_pattern;
#[path = "os/shared/write_new.rs"]
mod write_new;

pub use batch_limits::{
    check_get_batch, check_put_batch, clip_text, get_item_len, numbered_name, put_entry_len,
    split_batch, BATCH_HEADER_MAX, BATCH_MAX_BYTES, BATCH_MAX_FILES, BATCH_UNKNOWN_MARKER,
    ITEM_PATH_MAX, ITEM_TEXT_MAX,
};
pub use codec::{free_cores, pack_lz4, pack_raw, raw_len, unpack, BlockSender, MAX_RAW};
pub use codec::{read_frame, read_frame_with_tree_budget, write_frame};
pub use credit::{
    busy_message, charged, credit_cost, parse_busy, window_target, RecvWindow, SendCredit,
    StreamCount, CREDIT_CONNECTION_BUDGET, CREDIT_INITIAL, CREDIT_REQUEST_LIMIT, CREDIT_WINDOW_MAX,
};
pub use extension_parts::{
    append_duplicate_part, emit_duplicate_parts, emit_listing_parts, emit_meta_listing_parts,
};
pub use features::{
    lz4_switched_off, server_version, server_version_with, service_slots, ServerFeatures,
    LABEL_BATCH, LABEL_CREDIT, LABEL_LZ4, LABEL_RANGE, LABEL_STAGE,
};
pub use file_digest::{md5_cached, LocalReads};
pub use folder_key::{
    entry_part, failure_part, folder_key, known_prefix, omission_part, parse_known, run_digest,
    EntryKind, FolderKey, KeyEmit, KeyTree, KnownKey, Mode as KeyMode, RunKeys, Shape as KeyShape,
    MAX_KNOWN_KEYS,
};
pub use fs::{is_pseudo_dir, list_local, stat_local, walk_local, WalkCounter};
pub use key_tally::APP_TRASH_DIR_NAME;
pub use key_tally::{name_is_unusual, KeyTally};
pub use local_platform::available_space;
pub use local_platform::{file_stamp, path_stamp};
pub use node_codec::{TreeDecodeBudget, TREE_BUDGET_ERROR};
pub use ops_types::{digest, omission, query, UNSUPPORTED_EXTENSION};
pub use ordered_walk::{
    run as ordered_walk, walk_threads, Local as WalkLocal, Output as WalkOutput,
};
pub use promotion::validate_destination_root;
pub use put_tree::{BufferedTree, BufferedTreeReceiver, TreeManifestValidator};
pub use range_journal::Journal;
pub use range_ops::Lz4Inbound;
#[cfg(any(test, feature = "test-support"))]
pub use range_space::space_test_lock;
pub use range_space::{merge_in, overlaps};
pub use range_stage::{random_stage_name, Claim, Pending, RangeStages};
pub use relative_path::ValidatedRelativePath;
pub use run_digests::RunDigests;
#[cfg(target_os = "linux")]
pub use sandbox::restrict_filesystem;
pub use server::serve;
pub use server_session::{transfer_channel, RequestContext, ServerSession};
pub use session::{Inbound, Sink};
pub use transfer::{
    collect_local_tree, finish_local_tree_file, open_local_tree_file, LocalTreeEntry,
};
pub use transport_error::{parse_transport_error, transport_error_message};
pub use types::{
    has_link_aware_hash, BatchEntry, BatchItem, Frame, SearchSpec, WireMeta, WireNode, CHUNK,
    HASH_WALK_LINK_BOUNDARY, HASH_WALK_SERVER_VERSION, PROTO_VERSION, TRANSFER_FRAME_BACKLOG,
};
pub use types::{
    WireChange, WireDuplicateGroup, WireDuplicateItem, WireDuplicateSummary, WireHashEntry,
    WireHashMeta, WireOmission, WireReclaimProgress, WireTargetLimits,
};
pub use vault_error::{
    parse_vault_error, vault_error_message, VaultWireError, VAULT_LOCKED_WIRE,
};
pub use walk_filter::{
    host_patterns, HostFilter, WalkFilterSpec, WalkScope, MAX_FILTER_PATTERNS, MAX_FILTER_TEXT,
};
