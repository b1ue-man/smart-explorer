//! Default bodies of `Backend` methods that need no more than a constant or an
//! `Unsupported` answer, kept out of the trait file so it stays small. The
//! methods and their doc comments stay on the trait.
use std::io::{Read, Write};

use super::{
    Backend, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink, HashHit, SearchHit,
    VfsChangeBatch, VfsResult,
};

/// `Backend::item_id`: no stable identity.
pub(super) fn item_id(path: &str) -> VfsResult<Option<String>> {
    let _ = path;
    Ok(None)
}

/// `Backend::open_write_new`: no atomic exclusive-create writer.
pub(super) fn open_write_new(path: &str) -> VfsResult<Box<dyn Write + Send>> {
    let _ = path;
    super::meta::unsupported("backend has no atomic exclusive-create writer")
}

/// `Backend::server_copy_to_stage`: stream instead.
pub(super) fn server_copy_to_stage(
    src: &str,
    stage: &str,
    size: u64,
    cancel: &std::sync::atomic::AtomicBool,
) -> VfsResult<Option<u64>> {
    let _ = (src, stage, size, cancel);
    Ok(None)
}

/// `Backend::open_write_fresh`: use a copy stage.
pub(super) fn open_write_fresh(path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>> {
    let _ = (path, size);
    Ok(None)
}

/// `Backend::open_read_at`: cannot start mid-file.
pub(super) fn open_read_at(
    path: &str,
    id: Option<&str>,
    offset: u64,
) -> VfsResult<Option<Box<dyn Read + Send>>> {
    let _ = (path, id, offset);
    Ok(None)
}

/// `Backend::supports_range_reads`: no.
pub(super) fn supports_range_reads(path: &str) -> bool {
    let _ = path;
    false
}

/// `Backend::open_segmented_stage`: no backend until it writes at an offset.
pub(super) fn open_segmented_stage(
    stage: &str,
    size: u64,
) -> VfsResult<Option<Box<dyn super::ranges::SegmentedStage>>> {
    let _ = (stage, size);
    Ok(None)
}

/// `Backend::rename_no_replace`: no atomic no-replace primitive.
pub(super) fn rename_no_replace(src: &str, dst: &str) -> VfsResult<()> {
    let _ = (src, dst);
    super::meta::unsupported("backend has no atomic no-replace rename")
}

/// `Backend::discard_copy_stage`: ownership of the name cannot be proven.
pub(super) fn discard_copy_stage(stage: &str) -> VfsResult<()> {
    let _ = stage;
    super::meta::unsupported("stage cleanup unsupported")
}

/// `Backend::flow_key`: scheme and the address of this very handle.
pub(super) fn flow_key<B: Backend + ?Sized>(backend: &B, path: &str) -> String {
    let _ = path;
    format!(
        "{:?}@{:p}",
        backend.scheme(),
        backend as *const B as *const ()
    )
}

/// `Backend::flow_identity`: scheme and namespace identity, no addresses.
pub(super) fn flow_identity<B: Backend + ?Sized>(backend: &B, path: &str) -> String {
    let _ = path;
    format!("{:?}|{}", backend.scheme(), backend.namespace_identity())
}

/// `Backend::transfer_ceiling`: adaptive control only.
pub(super) fn transfer_ceiling(path: &str) -> Option<usize> {
    let _ = path;
    None
}

/// `Backend::open_read_id`: opens by path.
pub(super) fn open_read_id<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    id: Option<&str>,
) -> VfsResult<Box<dyn Read + Send>> {
    let _ = id;
    backend.open_read(path)
}

/// `Backend::remove_file_id`: deletes by path.
pub(super) fn remove_file_id<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    id: Option<&str>,
) -> VfsResult<()> {
    let _ = id;
    backend.remove_file(path)
}

/// `Backend::plan_dedupe_recursive`: nothing to remove.
pub(super) fn plan_dedupe_recursive(
    root: &str,
    keep: &dyn Fn(&str) -> bool,
) -> VfsResult<Vec<super::DedupeCandidate>> {
    let _ = (root, keep);
    Ok(Vec::new())
}

/// `Backend::dedupe_recursive`: plan, then apply the plan.
pub(super) fn dedupe_recursive<B: Backend + ?Sized>(
    backend: &B,
    root: &str,
    keep: &dyn Fn(&str) -> bool,
) -> VfsResult<usize> {
    let plan = backend.plan_dedupe_recursive(root, keep)?;
    backend.apply_dedupe_plan(&plan)
}

/// `Backend::change_root_id`: no change feed.
pub(super) fn change_root_id(root: &str) -> VfsResult<Option<String>> {
    let _ = root;
    Ok(None)
}

/// `Backend::current_change_cursor`: no change feed.
pub(super) fn current_change_cursor(root: &str) -> VfsResult<Option<String>> {
    let _ = root;
    Ok(None)
}

/// `Backend::changes_since`: every cursor is invalid, rebuild from a snapshot.
pub(super) fn changes_since(root: &str, cursor: &str) -> VfsResult<VfsChangeBatch> {
    let _ = (root, cursor);
    Ok(VfsChangeBatch {
        reset: true,
        ..Default::default()
    })
}

/// `Backend::get_tree`: no bulk tree transfer.
pub(super) fn get_tree(root: &str, dst: &std::path::Path) -> VfsResult<u64> {
    let _ = (root, dst);
    super::meta::unsupported("bulk tree transfer not supported")
}

/// `Backend::put_tree`: no bulk tree transfer.
pub(super) fn put_tree(src: &std::path::Path, root: &str) -> VfsResult<u64> {
    let _ = (src, root);
    super::meta::unsupported("bulk tree transfer not supported")
}

/// `Backend::batch_limits`: no batches.
pub(super) fn batch_limits(dir: &str) -> Option<BatchLimits> {
    let _ = dir;
    None
}

/// `Backend::put_batch`: no batch upload.
pub(super) fn put_batch(
    entries: &[BatchPut],
    data: &mut dyn Read,
) -> VfsResult<Vec<BatchPutOutcome>> {
    let _ = (entries, data);
    super::meta::unsupported("batch upload not supported")
}

/// `Backend::get_batch`: no batch download.
pub(super) fn get_batch(items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
    let _ = (items, sink);
    super::meta::unsupported("batch download not supported")
}

/// `Backend::search`: unsupported before any hit.
pub(super) fn search(
    root: &str,
    spec: &crate::agent_proto::SearchSpec,
    tx: crossbeam_channel::Sender<SearchHit>,
    cancel: &std::sync::atomic::AtomicBool,
) -> VfsResult<bool> {
    let _ = (root, spec, tx, cancel);
    Ok(false)
}

/// `Backend::walk_hashed`: unsupported before any entry.
pub(super) fn walk_hashed(
    root: &str,
    want_hash: bool,
    tx: crossbeam_channel::Sender<HashHit>,
    cancel: &std::sync::atomic::AtomicBool,
) -> VfsResult<bool> {
    let _ = (root, want_hash, tx, cancel);
    Ok(false)
}

/// `Backend::open_write_copy_stage_sized`: the size is not needed, the stage
/// of unknown length is used.
pub(super) fn open_write_copy_stage_sized<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    size: u64,
) -> VfsResult<Box<dyn Write + Send>> {
    let _ = size;
    backend.open_write_copy_stage(path)
}
