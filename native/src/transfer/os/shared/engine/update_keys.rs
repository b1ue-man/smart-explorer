//! Folder keys for the file rule (`update.rs`, whose submodule this is): a
//! folder of the source whose key equals the key of the same-named folder at
//! the target is the same folder for the rule, so nothing of it is listed at
//! the target and no file in it is compared.
//!
//! A key is formed where the files are (`agent_proto::folder_key`): a local
//! side runs the agent's walk in this process (`LocalKeyed::keys_of`), a
//! remote side asks its host for the keys of its tree (`hash_walk_keyed` with
//! `KeyedWalk::KeysOnly`: one key per folder crosses the connection, no
//! entry). The key covers exactly what the rule compares: per file the name,
//! the size and the modification time in milliseconds, plus kinds, hidden
//! flags and omissions. Equal keys therefore mean equal size and a time
//! distance of 0, which is "equal" for every window the rule uses; unequal
//! keys mean nothing but "look inside": those folders take the listing and
//! the comparison per file as before, and the subfolders whose keys are
//! equal are still left out.
//!
//! Keys are used where they save a connection: for a remote target. A local
//! target costs one `stat` per file, less than walking and hashing the whole
//! tree. A side without the keyed walk (older hosts, SFTP, FTP, WebDAV, SMB,
//! Drive) gives no keys, and the rule takes the way without them.
//!
//! The target tree may be far larger than the merged folder. Its walk is
//! given up once it has more than `FOLDERS_PER_SOURCE_FOLDER` folders per
//! source folder (plus `FOLDERS_SLACK`): one listing request per source
//! folder is cheaper then. The bound is an assumption about cost (a listing
//! is a round trip, a folder read on the host next to the data is not) and
//! decides only which way is taken, never the result.
use super::super::super::flow::{classify_error, Flow};
use super::super::super::flow_control::OpOutcome;
use super::super::super::walk_listers::native;
use super::super::view::Side;
use crate::vfs::{Backend, HashWalkRequest, KeyedWalk, KeyedWalkItem};
use crossbeam_channel::RecvTimeoutError;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

type Key = [u8; 32];
type KeyMap = HashMap<String, Key>;

/// Target folders allowed per source folder before the key walk is dropped.
const FOLDERS_PER_SOURCE_FOLDER: usize = 16;
/// Target folders allowed beyond that, so a small merge still gets keys.
const FOLDERS_SLACK: usize = 1024;
/// How often a waiting reader looks at the stop flags.
const POLL: Duration = Duration::from_millis(50);

/// One side of a comparison: where it lives, the folder to key, and the flow
/// a remote walk takes its permit from (`None` = no permit, as the preview).
#[derive(Clone, Copy)]
pub(in super::super) struct Place<'a> {
    pub side: Side<'a>,
    pub root: &'a str,
    pub flow: Option<&'a Arc<Flow>>,
}

/// The folders (paths below the compared roots, `""` = the root itself) whose
/// keys are equal on both sides and whose parent's key is not: everything
/// below them is equal too.
pub(in super::super) struct Keys {
    top: HashSet<String>,
}

impl Keys {
    /// Whether the folder `dir` is, or lies below, an equal folder.
    pub(in super::super) fn covers(&self, dir: &str) -> bool {
        let mut current = dir;
        loop {
            if self.top.contains(current) {
                return true;
            }
            if current.is_empty() {
                return false;
            }
            current = parent(current);
        }
    }

    fn of(source: &KeyMap, target: &KeyMap) -> Option<Self> {
        let equal: HashSet<&str> = source
            .iter()
            .filter(|(rel, key)| target.get(*rel) == Some(*key))
            .map(|(rel, _)| rel.as_str())
            .collect();
        let top: HashSet<String> = equal
            .iter()
            .filter(|rel| !has_equal_ancestor(&equal, rel))
            .map(|rel| (*rel).to_string())
            .collect();
        (!top.is_empty()).then_some(Self { top })
    }
}

fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn has_equal_ancestor(equal: &HashSet<&str>, rel: &str) -> bool {
    let mut current = rel;
    while !current.is_empty() {
        current = parent(current);
        if equal.contains(current) {
            return true;
        }
    }
    false
}

/// What the target walk may take, from the source walk running beside it.
#[derive(Default)]
struct Bound {
    source_folders: AtomicUsize,
    source_done: AtomicBool,
    /// The source gave no keys: the comparison is lost, stop the target walk.
    source_failed: AtomicBool,
}

impl Bound {
    fn exceeded(&self, target_folders: usize) -> bool {
        if self.source_failed.load(Ordering::Acquire) {
            return true;
        }
        self.source_done.load(Ordering::Acquire)
            && target_folders
                > self
                    .source_folders
                    .load(Ordering::Acquire)
                    .saturating_mul(FOLDERS_PER_SOURCE_FOLDER)
                    .saturating_add(FOLDERS_SLACK)
    }
}

/// The folders of `source` and `target` with equal keys, `None` when keys do
/// not apply (local target, a side without the keyed walk, a failed or
/// canceled walk, a target tree too large to walk) or no folder is equal.
/// Both trees are keyed at the same time.
pub(in super::super) fn compare(
    source: Place<'_>,
    target: Place<'_>,
    cancel: &AtomicBool,
) -> Option<Keys> {
    if target.side.is_local() {
        return None;
    }
    let bound = Bound::default();
    let (source_keys, target_keys) = std::thread::scope(|scope| {
        let source_walk = scope.spawn(|| {
            let keys = side_keys(source, cancel, None);
            match &keys {
                Some(keys) => {
                    bound.source_folders.store(keys.len(), Ordering::Release);
                    bound.source_done.store(true, Ordering::Release);
                }
                None => bound.source_failed.store(true, Ordering::Release),
            }
            keys
        });
        let target_keys = side_keys(target, cancel, Some(&bound));
        (source_walk.join().ok().flatten(), target_keys)
    });
    Keys::of(&source_keys?, &target_keys?)
}

fn side_keys(place: Place<'_>, cancel: &AtomicBool, bound: Option<&Bound>) -> Option<KeyMap> {
    match place.side {
        Side::Local => {
            let root = native(place.root);
            let keys = crate::agent_proto::LocalKeyed::keys_of(
                root.to_str()?,
                crate::agent_proto::digest::NONE,
                cancel,
            )
            .ok()?;
            Some(keys.into_iter().collect())
        }
        Side::Remote(backend) => remote_keys(backend, place, cancel, bound),
    }
}

/// The keys of a remote tree from its host. `None` for a host without the
/// keyed walk, a failed walk, a stop and a walk that outgrew `bound`.
fn remote_keys(
    backend: &dyn Backend,
    place: Place<'_>,
    cancel: &AtomicBool,
    bound: Option<&Bound>,
) -> Option<KeyMap> {
    let permit = match place.flow {
        Some(flow) => Some(flow.acquire_meta(cancel)?),
        None => None,
    };
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let stop = AtomicBool::new(false);
    let mut keys = KeyMap::new();
    let ran = std::thread::scope(|scope| {
        let stop = &stop;
        let walk = scope.spawn(move || {
            crate::vfs::hash_walk_keyed(backend, place.root, request, KeyedWalk::KeysOnly, tx, stop)
        });
        loop {
            match rx.recv_timeout(POLL) {
                Ok(KeyedWalkItem::Key { rel, key, .. }) => {
                    keys.insert(rel, key);
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if cancel.load(Ordering::Acquire) || bound.is_some_and(|b| b.exceeded(keys.len())) {
                stop.store(true, Ordering::Release);
            }
        }
        walk.join()
    });
    if let Some(permit) = permit {
        // A host without the walk, or a walk this side stopped, did no work.
        match &ran {
            Ok(Ok(true)) if !stop.load(Ordering::Acquire) => permit.finish(OpOutcome::Done),
            Ok(Err(error)) if !stop.load(Ordering::Acquire) => permit.finish(classify_error(error)),
            _ => permit.abandon(),
        }
    }
    match ran {
        Ok(Ok(true)) if !stop.load(Ordering::Acquire) => Some(keys),
        _ => None,
    }
}
