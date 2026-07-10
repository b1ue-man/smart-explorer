use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::snapshot::WalkFilter;
use super::snapshot_hash::{md5_hex_to_u64, HashMode};
use super::types::{Sig, Tree};

const MAX_WALK_NODES: u64 = 1_000_000;
const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;

/// Build a signature tree from the agent's bounded one-pass server-side walk.
/// `Some` means the agent handled it; `None` asks the caller to fall back.
pub(super) fn walk_hashed_via_agent(
    backend: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash_mode: HashMode,
) -> io::Result<Option<Tree>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let want_hash = matches!(hash_mode, HashMode::Full | HashMode::FullFresh);
    let (sender, receiver) = crossbeam_channel::bounded::<crate::vfs::HashHit>(1024);
    let mut tree = Tree::new();
    let mut nodes = 0u64;
    let mut text_bytes = 0u64;
    let mut failure = None;
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(|| backend.walk_hashed(root, want_hash, sender, cancel));
        for hit in receiver.iter() {
            if cancel.load(Ordering::Relaxed) {
                failure.get_or_insert_with(canceled);
                continue;
            }
            if failure.is_some() {
                continue;
            }
            nodes = nodes.saturating_add(1);
            text_bytes = text_bytes.saturating_add(hit.rel.len() as u64);
            if nodes > MAX_WALK_NODES || text_bytes > MAX_WALK_TEXT_BYTES {
                failure = Some(invalid("agent sync tree exceeds its collection budget"));
                continue;
            }
            if let Err(error) = crate::agent_proto::ValidatedRelativePath::parse(&hit.rel) {
                failure = Some(error);
                continue;
            }
            if hit.is_dir || filter.ignore.is_match(&hit.rel) {
                continue;
            }
            let hidden = hit
                .rel
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with('.'));
            if (!filter.include_hidden && hidden) || !filter.size_age_ok(hit.size, hit.mtime_ms) {
                continue;
            }
            let checksum = hit.md5.as_deref().map(md5_hex_to_u64).unwrap_or(0);
            if want_hash && checksum == 0 {
                failure = Some(invalid(format!(
                    "agent returned a missing or invalid checksum for {}",
                    hit.rel
                )));
                continue;
            }
            let rel = hit.rel;
            if tree
                .insert(
                    rel.clone(),
                    Sig {
                        size: hit.size,
                        mtime_ms: hit.mtime_ms,
                        hash: checksum,
                    },
                )
                .is_some()
            {
                failure = Some(invalid(format!(
                    "agent returned duplicate sync path: {rel}"
                )));
            }
        }
        worker.join()
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let ran = match outcome {
        Ok(Ok(ran)) => ran,
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(io::Error::other("agent sync walk worker panicked")),
    };
    if let Some(error) = failure {
        Err(error)
    } else if !ran {
        if nodes == 0 {
            Ok(None)
        } else {
            Err(invalid(
                "backend reported hash walk unsupported after streaming entries",
            ))
        }
    } else {
        Ok(Some(tree))
    }
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent sync walk canceled")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
