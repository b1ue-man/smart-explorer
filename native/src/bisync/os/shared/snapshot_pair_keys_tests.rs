//! The pair's key phases (E10, first comparison without stored keys) with
//! real agent servers: phase 1 sends keys and no item, phase 2 the items of
//! each distinct unequal folder once; a local side's keys, formed by its one
//! snapshot walk, spare the remote side every equal folder; every snapshot
//! equals the walk without keys.
use super::super::omissions::{OmissionKind, SyncOmissions};
use super::super::snapshot::walk_snapshot_observed;
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::WalkContext;
use super::super::snapshot_hash::HashMode;
use super::super::snapshot_keyed::{KeyFiles, KeyedTree};
use super::super::snapshot_pair_side::{walk_remote, Walker};
use super::super::snapshot_types::DirSet;
use super::super::sync_overload::Progress;
use super::super::types::{BisyncOptions, Tree};
use super::{PairKeys, SideSetup};
use crate::agent::AgentBackend;
use crate::vfs::{self as vfs, Backend, HashWalkRequest, KeyedWalk, KeyedWalkItem};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// A client of a real agent server, seen as a remote side
/// (`abgleich_task_agent_fixture.rs`).
fn agent() -> AgentBackend {
    super::super::abgleich_task_agent_fixture::remote_agent()
}

/// A walker that counts the items (not the keys) crossing the connection.
struct Counting<'b> {
    backend: &'b AgentBackend,
    root: String,
    items: AtomicU64,
}

impl Walker for Counting<'_> {
    fn walk(
        &self,
        walk: KeyedWalk<'_>,
        take: &mut dyn FnMut(KeyedWalkItem) -> io::Result<()>,
    ) -> io::Result<bool> {
        let request = HashWalkRequest {
            algorithm: None,
            min_bytes: 0,
            meta: true,
            fresh: false,
            filter: None,
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = AtomicBool::new(false);
        let ran = vfs::hash_walk_keyed(self.backend, &self.root, request, walk, tx, &cancel)?;
        for item in rx.try_iter() {
            if matches!(item, KeyedWalkItem::Item(_)) {
                self.items.fetch_add(1, Ordering::SeqCst);
            }
            take(item)?;
        }
        Ok(ran)
    }
}

#[derive(Debug, PartialEq)]
struct Side {
    tree: Tree,
    dirs: Vec<String>,
    omissions: Vec<(String, OmissionKind)>,
}

/// One side's snapshot: through the key phases (`run` gets the context),
/// or (`run = None`) through the walk without keys.
fn side(
    backend: &dyn Backend,
    root: &str,
    pair: Option<(&PairKeys, usize)>,
    run: impl FnOnce(&WalkContext<'_>) -> Option<KeyedTree>,
) -> (Side, Option<KeyedTree>) {
    let cancel = AtomicBool::new(false);
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let dirs = Mutex::new(DirSet::new());
    let ctx = WalkContext {
        be: backend,
        root,
        cancel: &cancel,
        filter: &filter,
        hash: HashMode::None,
        prev: None,
        allow_duplicate_files: false,
        omissions: Some(&omissions),
        duplicates: None,
        filtered: None,
        dirs: Some(&dirs),
        opts: BisyncOptions::default(),
        limits: super::super::SyncLimits::for_memory(None),
        nodes: AtomicU64::new(0),
        text_bytes: AtomicU64::new(0),
        reads: None,
        progress: Progress::default(),
        log: None,
        listed: None,
        host_listed: true,
        local: None,
        key_files: None,
        pair,
        local_keys: None,
    };
    let keyed = run(&ctx);
    let tree = match &keyed {
        Some(keyed) => keyed.tree.clone(),
        None => super::super::snapshot_agent::walk_snapshot_via_agent(&ctx)
            .unwrap()
            .expect("the agent walks next to the data"),
    };
    drop(ctx);
    let side = Side {
        tree,
        dirs: dirs.into_inner().unwrap().into_iter().collect(),
        omissions: omissions
            .into_inner()
            .unwrap()
            .paths()
            .map(|(path, kind, _)| (path.to_string(), kind))
            .collect(),
    };
    (side, keyed)
}

fn file(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
}

/// 12 folders, 23 items: the root (2 files, 6 folders), `leer`, and five
/// `d*` with `f.txt` and `inner/g.txt` of distinct sizes.
fn tree(root: &Path) {
    std::fs::create_dir_all(root.join("leer")).unwrap();
    file(&root.join("r1.txt"), "eins");
    file(&root.join("r2.txt"), "zwei");
    for index in 0..5 {
        let dir = root.join(format!("d{index}"));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        file(&dir.join("f.txt"), &"f".repeat(index + 1));
        file(&dir.join("inner").join("g.txt"), &"g".repeat(index + 10));
    }
}

fn root_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn files(cache: &Path, name: &str) -> KeyFiles {
    KeyFiles {
        own: cache.join(format!("{name}.sefk")),
        peer: None,
    }
}

fn remote_setup() -> SideSetup {
    SideSetup {
        remote: true,
        eligible: true,
        stored: Default::default(),
    }
}

#[test]
fn abgleich_task_first_comparison_sends_keys_then_only_unequal_folders() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let (a, b) = (data.path().join("a"), data.path().join("b"));
    tree(&a);
    tree(&b);
    file(&b.join("d2").join("f.txt"), "anders als in a");
    let (agent_a, agent_b) = (agent(), agent());
    let (root_a, root_b) = (root_of(&a), root_of(&b));
    let pair = PairKeys::new([remote_setup(), remote_setup()]).expect("no side stored keys");
    let walker_a = Counting {
        backend: &agent_a,
        root: root_a.clone(),
        items: AtomicU64::new(0),
    };
    let walker_b = Counting {
        backend: &agent_b,
        root: root_b.clone(),
        items: AtomicU64::new(0),
    };
    let (files_a, files_b) = (files(cache.path(), "a"), files(cache.path(), "b"));
    let ((keyed_a, walked_a), (keyed_b, walked_b)) = std::thread::scope(|scope| {
        let side_a = scope.spawn(|| {
            side(&agent_a, &root_a, Some((&pair, 0)), |ctx| {
                walk_remote(ctx, (&pair, 0), &files_a, &walker_a).unwrap()
            })
        });
        let side_b = scope.spawn(|| {
            side(&agent_b, &root_b, Some((&pair, 1)), |ctx| {
                walk_remote(ctx, (&pair, 1), &files_b, &walker_b).unwrap()
            })
        });
        (side_a.join().unwrap(), side_b.join().unwrap())
    });
    let (full_a, _) = side(&agent_a, &root_a, None, |_| None);
    let (full_b, _) = side(&agent_b, &root_b, None, |_| None);
    assert_eq!(keyed_a, full_a);
    assert_eq!(keyed_b, full_b);
    assert!(
        walked_a.is_some() && walked_b.is_some(),
        "both sides replayed"
    );
    // Phase 1 sent no item; phase 2 every distinct folder once: all twelve
    // of one side (23 items) plus B's own root (8) and d2 (2).
    let (sent_a, sent_b) = (
        walker_a.items.load(Ordering::SeqCst),
        walker_b.items.load(Ordering::SeqCst),
    );
    assert_eq!(sent_a + sent_b, 33, "a: {sent_a}, b: {sent_b}");
    assert!(sent_a > 0 && sent_b > 0, "both sides fetched at once");
    assert!(
        files_a.own.exists() && files_b.own.exists(),
        "both sides store their keys"
    );
}

/// The snapshot tree of the local side `root`, as side 0 of `pair`.
fn local_walk(backend: &dyn Backend, root: &str, pair: Option<&PairKeys>) -> Tree {
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions {
        cross_mounts: true,
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let pair = pair.map(|pair| (pair, 0));
    let flags = (false, false);
    walk_snapshot_observed(
        backend,
        root,
        &cancel,
        &filter,
        HashMode::None,
        None,
        flags.0,
        flags.1,
        opts,
        None,
        (None, pair),
    )
    .unwrap()
    .tree
}

#[test]
fn abgleich_task_local_side_keys_spare_the_remote_side_equal_folders() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let (a, b) = (data.path().join("a"), data.path().join("b"));
    tree(&a);
    tree(&b);
    let remote = agent();
    let (root_a, root_b) = (root_of(&a), root_of(&b));
    for changed in [false, true] {
        if changed {
            file(&b.join("d2").join("f.txt"), "jetzt anders");
        }
        let local_setup = SideSetup {
            remote: false,
            eligible: true,
            stored: Default::default(),
        };
        let pair = PairKeys::new([local_setup, remote_setup()]).unwrap();
        assert!(pair.takes_part(0) && pair.takes_part(1));
        let walker = Counting {
            backend: &remote,
            root: root_b.clone(),
            items: AtomicU64::new(0),
        };
        let own = files(cache.path(), if changed { "b2" } else { "b1" });
        let local = vfs::LocalBackend::new(&root_a);
        let (local_tree, (keyed, walked)) = std::thread::scope(|scope| {
            // The local side's own snapshot walk offers its keys.
            let local_side = scope.spawn(|| local_walk(&local, &root_a, Some(&pair)));
            let remote_side = side(&remote, &root_b, Some((&pair, 1)), |ctx| {
                walk_remote(ctx, (&pair, 1), &own, &walker).unwrap()
            });
            (local_side.join().unwrap(), remote_side)
        });
        let (full, _) = side(&remote, &root_b, None, |_| None);
        assert_eq!(keyed, full);
        assert_eq!(
            local_tree,
            local_walk(&local, &root_a, None),
            "the local snapshot is the one of its walk without keys"
        );
        assert!(walked.is_some());
        let expected = if changed { 10 } else { 0 };
        assert_eq!(
            walker.items.load(Ordering::SeqCst),
            expected,
            "equal folders came from the local side (changed: {changed})"
        );
    }
}
