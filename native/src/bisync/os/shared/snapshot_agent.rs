//! A remote snapshot side from one walk next to the data (Share host
//! `hash_walk_v1`, agent or service `ext-v1`): one request instead of a
//! status and a listing per folder. The items are checked as the walk over
//! listings checks them (`snapshot_host_walk.rs`), so the result is the same.
//!
//! The walk over listings does not test mount boundaries on a remote side
//! either (`snapshot_policy::protected` only for local sides), so this walk
//! serves `cross_mounts = false` as well. Hidden entries without a leading
//! dot need the listing attributes (`HashWalkRequest::meta`); a side without
//! them reads its listings instead. A free provider hash (`NativeOnly`)
//! comes only with a listing, so that mode reads the listings as well.
use super::snapshot_dir::WalkContext;
use super::snapshot_hash::HashMode;
use super::snapshot_host_walk::Collector;
use super::types::{BisyncOptions, Tree};
use crate::vfs::{HashWalkItem, HashWalkRequest};
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
    let request = HashWalkRequest {
        algorithm: want_hash.then_some(crate::analytics::HashAlgorithm::Md5),
        min_bytes: 0,
        meta: !ctx.filter.include_hidden,
    };
    let mut collector = Collector::new(ctx);
    let primary = stream(&mut collector, |sender| {
        crate::vfs::hash_walk(backend, ctx.root, request, sender, ctx.cancel)
    });
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
        let legacy = stream(&mut collector, |sender| {
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
                worker
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("legacy hash walk worker panicked")))
            })
        });
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

/// Runs `walk` on its own thread and hands every item to `collector`; the
/// first item it refuses stops the walk (its receiver goes away).
fn stream(
    collector: &mut Collector<'_, '_>,
    walk: impl FnOnce(crossbeam_channel::Sender<HashWalkItem>) -> io::Result<bool> + Send,
) -> io::Result<bool> {
    let (sender, receiver) = crossbeam_channel::bounded(1024);
    let mut failure = None;
    let walked = std::thread::scope(|scope| {
        let worker = scope.spawn(move || walk(sender));
        for item in receiver.iter() {
            if let Err(error) = collector.item(item) {
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
pub(super) fn walk_hashed_via_agent(
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
