//! The capability text the background service announces for the backend it
//! serves (moved out of `backend_server.rs`; the labels are unchanged plus
//! `content_search_v1`).
use crate::agent_proto;
use crate::vfs::BackendHandle;

/// The version this service announces for `backend`: batches only when the
/// peer moves them itself (never emulated file by file), and no more
/// transfer slots than the peer admits. A Share peer's root is its export
/// list, which holds no files, so its batches are asked for host-wide
/// (Z-P1); each batch request is still checked against its own folder.
pub(super) fn service_version(backend: &BackendHandle) -> String {
    let root = backend.root_display();
    let batch = crate::vfs::host_batch_limits(&**backend).is_some();
    let slots = agent_proto::service_slots(backend.transfer_ceiling(&root));
    // A single file as a packet of one (Z-P4), where the peer offers it.
    let single = if batch && crate::vfs::single_put(&**backend) {
        ".single-put-v1"
    } else {
        ""
    };
    // Packet entries keep their time where the peer sets it.
    let times = if batch && crate::vfs::batch_put_times(&**backend) {
        ".batch-mtime-v1"
    } else {
        ""
    };
    // `range-v1` only for a backend that reads ranges (`backend_ranges`);
    // `ListMeta` … `StatMeta`: `backend_hash.rs`; `origin-v1`: below (E4d);
    // `content_search_v1`: `backend_content.rs` (every backend: where the peer
    // does not search itself, the worker reads the files through it).
    format!(
        "{}{}.capacity-v1.folder-reservation-v1.file-identity-v1.list-meta-v1.walk-keys-v1.meta-list-v1.origin-v1.{content}{single}{times} worker",
        agent_proto::server_version_with(batch, slots),
        super::backend_ranges::labels(backend),
        content = agent_proto::LABEL_CONTENT_SEARCH,
    )
}
