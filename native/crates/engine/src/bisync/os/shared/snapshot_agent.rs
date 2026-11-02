//! A remote snapshot side from one walk next to the data (Share host
//! `hash_walk_v1`, agent or service `ext-v1`): one request instead of a
//! status and a listing per folder. The items are checked as the walk over
//! listings checks them, so the result is the same.
//!
//! A side that sends listing attributes (`HashWalkRequest::meta`: Share
//! `hash_walk_meta_v1`, agent and service `walk-meta-v1`) is asked for them
//! always: its walk sends every folder as one unbroken run, so each folder
//! is checked as soon as it is complete (`snapshot_host_stream.rs`: progress
//! and early copies during the walk, one folder in memory). A side without
//! them is asked without attributes where hidden entries count anyway; its
//! items are collected and checked after the walk (`snapshot_host_walk.rs`).
//!
//! A side with folder keys (`snapshot_keyed.rs`) is asked for them first:
//! folders whose key this or the other side of the pair stored come as one
//! mark each and are replayed from the stored keys.
//!
//! Every request carries the job's filter and, in the checksum mode, asks
//! for digests read from the files now (`HashWalkRequest::fresh`,
//! `filter`): a side that applies the filter sends files it leaves out
//! without a digest and does not enter folders it leaves out, which the
//! check here leaves out as well (`agent_proto::walk_filter`).
//!
//! The walk over listings does not test mount boundaries on a remote side
//! either (`snapshot_policy::protected` only for local sides), so this walk
//! serves `cross_mounts = false` as well. Hidden entries without a leading
//! dot need the listing attributes; a side without them reads its listings
//! instead. A free provider hash (`NativeOnly`) comes only with a listing,
//! so that mode reads the listings as well.
use super::snapshot_dir::WalkContext;
use super::snapshot_hash::HashMode;
use super::snapshot_host_stream::Stream;
use super::snapshot_host_walk::Collector;
use super::snapshot_pair_side::Walker;
use super::types::{BisyncOptions, Tree};
use crate::vfs::{HashWalkItem, HashWalkRequest, KeyedWalk, KeyedWalkItem};
use std::io;

/// The tree of `ctx.root` (the context collects folders, filtered files and
/// omissions), or `None` for the walk over listings to take over before
/// anything was recorded.
pub(super) fn walk_snapshot_via_agent(ctx: &WalkContext<'_>) -> io::Result<Option<Tree>> {
    let backend = ctx.be;
    if backend.is_local()
        || backend.has_duplicate_file_names()
        || !matches!(ctx.hash, HashMode::None | HashMode::FullFresh)
    {
        return Ok(None);
    }
    super::transfer_stream::check(ctx.cancel)?;
    let want_hash = ctx.hash == HashMode::FullFresh;
    let algorithm = want_hash.then_some(crate::analytics::HashAlgorithm::Md5);
    // The checksum mode asks for digests read now; the filter is the job's.
    let meta_request = HashWalkRequest {
        algorithm,
        min_bytes: 0,
        meta: true,
        fresh: want_hash,
        filter: ctx.filter.host_spec(),
    };
    if let Some(files) = &ctx.key_files {
        let request = meta_request.clone();
        let walker = HostWalker { ctx, request };
        // The pair's key phases first (no stored keys yet), then the walk
        // with the stored keys of both sides as known.
        let mut keyed = None;
        if let Some((pair, side)) = ctx.pair.filter(|(pair, side)| pair.takes_part(*side)) {
            keyed = super::snapshot_pair_side::walk_remote(ctx, (pair, side), files, &walker)?;
        }
        if keyed.is_none() {
            keyed = super::snapshot_keyed::walk(ctx, files, |take, known| {
                walker.walk(KeyedWalk::Marks { known }, take)
            })?;
        }
        if let Some(keyed) = keyed {
            super::run_log::note(
                "Scan",
                &format!(
                    "{}: {} Ordner gleich ihrem Ordnerschlüssel (nicht übertragen), {} Ordner übertragen",
                    ctx.root, keyed.reused, keyed.sent
                ),
            );
            return Ok(Some(keyed.tree));
        }
    }
    let mut streamed = Stream::new(ctx);
    let with_meta = stream(
        |item| streamed.item(item),
        |sender| crate::vfs::hash_walk(backend, ctx.root, meta_request, sender, ctx.cancel),
    );
    match with_meta {
        Ok(true) => {
            super::transfer_stream::check(ctx.cancel)?;
            return streamed.finish_tree().map(Some);
        }
        Ok(false) if !streamed.is_empty() => {
            return Err(invalid("hash walk refused after emitting entries"))
        }
        Ok(false) => {}
        Err(error) if error.to_string() == crate::agent_proto::HASH_WALK_LINK_BOUNDARY => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    }
    // Without attributes hidden entries cannot be told apart.
    if !ctx.filter.include_hidden {
        return Ok(None);
    }
    let request = HashWalkRequest {
        algorithm,
        min_bytes: 0,
        meta: false,
        fresh: want_hash,
        filter: None,
    };
    let mut collector = Collector::new(ctx);
    let primary = stream(
        |item| collector.item(item),
        |sender| crate::vfs::hash_walk(backend, ctx.root, request, sender, ctx.cancel),
    );
    let primary = match primary {
        Ok(ran) => ran,
        Err(error) if error.to_string() == crate::agent_proto::HASH_WALK_LINK_BOUNDARY => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if !primary {
        if !collector.is_empty() {
            return Err(invalid("hash walk refused after emitting entries"));
        }
        // The walk of `sync-links-v1` agents reports no omissions and passes
        // special files by: only where it always served (across mounts,
        // hidden entries included).
        if !(ctx.opts.cross_mounts && ctx.filter.include_hidden) {
            return Ok(None);
        }
        let legacy = stream(
            |item| collector.item(item),
            |sender| {
                let (hits, received) = crossbeam_channel::bounded::<crate::vfs::HashHit>(1024);
                std::thread::scope(|scope| {
                    let worker =
                        scope.spawn(|| backend.walk_hashed(ctx.root, want_hash, hits, ctx.cancel));
                    for hit in received.iter() {
                        let item = HashWalkItem::Entry(crate::vfs::HashWalkEntry {
                            rel: hit.rel,
                            is_dir: hit.is_dir,
                            size: hit.size,
                            mtime_ms: hit.mtime_ms,
                            digest: hit.md5,
                            meta: None,
                        });
                        if sender.send(item).is_err() {
                            break;
                        }
                    }
                    drop(received);
                    worker.join().unwrap_or_else(|_| {
                        Err(io::Error::other("legacy hash walk worker panicked"))
                    })
                })
            },
        );
        match legacy {
            Ok(false) if collector.is_empty() => return Ok(None),
            Ok(false) => return Err(invalid("legacy hash walk refused after emitting entries")),
            Ok(true) => {}
            Err(error) if error.to_string() == crate::agent_proto::HASH_WALK_LINK_BOUNDARY => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        }
    }
    super::transfer_stream::check(ctx.cancel)?;
    collector.replay().map(Some)
}

/// The keyed walks of a remote side next to its data.
struct HostWalker<'c, 'a> {
    ctx: &'c WalkContext<'a>,
    request: HashWalkRequest,
}

impl super::snapshot_pair_side::Walker for HostWalker<'_, '_> {
    fn walk(
        &self,
        walk: KeyedWalk<'_>,
        take: &mut dyn FnMut(KeyedWalkItem) -> io::Result<()>,
    ) -> io::Result<bool> {
        let (ctx, request) = (self.ctx, self.request.clone());
        stream(take, |sender| {
            crate::vfs::hash_walk_keyed(ctx.be, ctx.root, request, walk, sender, ctx.cancel)
        })
    }
}

/// Runs `walk` on its own thread and hands every item to `take`; the first
/// item it refuses stops the walk (its receiver goes away).
fn stream<T: Send>(
    mut take: impl FnMut(T) -> io::Result<()>,
    walk: impl FnOnce(crossbeam_channel::Sender<T>) -> io::Result<bool> + Send,
) -> io::Result<bool> {
    let (sender, receiver) = crossbeam_channel::bounded(1024);
    let mut failure = None;
    let walked = std::thread::scope(|scope| {
        let worker = scope.spawn(move || walk(sender));
        for item in receiver.iter() {
            if let Err(error) = take(item) {
                failure = Some(error);
                break;
            }
        }
        drop(receiver);
        worker.join()
    });
    if let Some(error) = failure {
        return Err(error);
    }
    match walked {
        Ok(result) => result,
        Err(_) => Err(io::Error::other("hash walk worker panicked")),
    }
}

/// The tree of a whole remote side through its walk next to the data, with
/// hidden entries and across mounts (`None` = that side has no such walk).
pub fn walk_hashed_via_agent(
    backend: &dyn crate::vfs::Backend,
    root: &str,
    cancel: &std::sync::atomic::AtomicBool,
    filter: &super::snapshot::WalkFilter,
    hash: HashMode,
) -> io::Result<Option<Tree>> {
    let opts = BisyncOptions {
        cross_mounts: true,
        ..Default::default()
    };
    super::snapshot::with_walk_context(backend, root, cancel, filter, hash, opts, |ctx| {
        walk_snapshot_via_agent(ctx)
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(all(test, unix))]
#[path = "abgleich_task_walk_filter_tests.rs"]
mod walk_filter_tests;
