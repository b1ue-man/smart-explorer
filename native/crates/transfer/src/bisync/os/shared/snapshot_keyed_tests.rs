//! Folder keys in the sync comparison (E10) through the real agent server:
//! the first walk sends every folder and stores its keys; an unchanged side
//! sends no item at all; a changed file sends only the folders on its path;
//! a side equal to what the other side stored sends nothing either. Every
//! snapshot (files, filtered files, folders, omissions) and every plan
//! equals the one of the walk without keys.
use super::super::omissions::{OmissionKind, SyncOmissions};
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::WalkContext;
use super::super::snapshot_hash::HashMode;
use super::super::snapshot_types::DirSet;
use super::super::sync_overload::Progress;
use super::super::types::{Baseline, BisyncOptions, Sig, Tree};
use super::{walk, KeyFiles, KeyedTree};
use crate::agent::AgentBackend;
use crate::vfs::{self as vfs, HashWalkRequest, KeyedWalkItem};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// A client of a real agent server, seen as a remote side
/// (`abgleich_task_agent_fixture.rs`).
fn agent() -> AgentBackend {
    super::super::abgleich_task_agent_fixture::remote_agent()
}

/// What one side's walk gave.
#[derive(Debug, PartialEq)]
struct Side {
    tree: Tree,
    filtered: Tree,
    dirs: Vec<String>,
    omissions: Vec<(String, OmissionKind)>,
}

/// The side through the keyed walk (`files`), or without keys (`None`);
/// with keys also what crossed the connection and the walk's counts.
fn read(
    backend: &AgentBackend,
    root: &str,
    files: Option<&KeyFiles>,
    hash: HashMode,
) -> (Side, Option<(KeyedTree, u64)>) {
    let cancel = AtomicBool::new(false);
    let globs = {
        let mut builder = globset::GlobSetBuilder::new();
        builder.add(globset::Glob::new("**/*.tmp").unwrap());
        builder.build().unwrap()
    };
    let filter = WalkFilter::basic(false, &globs);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let filtered = Mutex::new(Tree::new());
    let dirs = Mutex::new(DirSet::new());
    let ctx = WalkContext {
        be: backend,
        root,
        cancel: &cancel,
        filter: &filter,
        hash,
        prev: None,
        allow_duplicate_files: false,
        omissions: Some(&omissions),
        duplicates: None,
        filtered: Some(&filtered),
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
        pair: None,
        local_keys: None,
    };
    let (tree, keyed) = match files {
        None => (
            super::super::snapshot_agent::walk_snapshot_via_agent(&ctx)
                .unwrap()
                .expect("the agent walks next to the data"),
            None,
        ),
        Some(files) => {
            let mut sent_items = 0u64;
            let request = HashWalkRequest {
                algorithm: (hash == HashMode::FullFresh)
                    .then_some(crate::analytics::HashAlgorithm::Md5),
                min_bytes: 0,
                meta: true,
                fresh: false,
                filter: None,
            };
            let keyed = walk(&ctx, files, |take, known| {
                let (tx, rx) = crossbeam_channel::unbounded();
                let ran = vfs::hash_walk_keyed(
                    backend,
                    root,
                    request,
                    vfs::KeyedWalk::Marks { known },
                    tx,
                    &cancel,
                )?;
                for item in rx.try_iter() {
                    if matches!(item, KeyedWalkItem::Item(_)) {
                        sent_items += 1;
                    }
                    take(item)?;
                }
                Ok(ran)
            })
            .unwrap()
            .expect("the agent offers folder keys");
            (keyed.tree.clone(), Some((keyed, sent_items)))
        }
    };
    drop(ctx);
    let side = Side {
        tree,
        filtered: filtered.into_inner().unwrap(),
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

fn set_old(path: &Path, seconds: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

/// Ten folders with files, a hidden file, a filtered file, an empty folder
/// and (Unix) a link; every file with a fixed time.
fn tree(root: &Path) {
    std::fs::create_dir_all(root.join("leer")).unwrap();
    for folder in 0..10 {
        let dir = root.join(format!("d{folder}")).join("inner");
        std::fs::create_dir_all(&dir).unwrap();
        for file in 0..3 {
            let path = dir.join(format!("f{file}.txt"));
            // Every folder's files differ in size: ten distinct keys.
            let content = format!("{folder}-{file}-{}", "x".repeat(folder));
            std::fs::write(&path, content).unwrap();
            set_old(&path, 1_700_000_000 + file);
        }
    }
    for (name, content) in [(".versteckt", "h"), ("a.txt", "abc"), ("weg.tmp", "t")] {
        let path = root.join(name);
        std::fs::write(&path, content).unwrap();
        set_old(&path, 1_700_000_100);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("a.txt"), root.join("link")).unwrap();
}

fn files(cache: &Path, side: &str, peer: Option<&str>) -> KeyFiles {
    KeyFiles {
        own: cache.join(format!("{side}.sefk")),
        peer: peer.map(|peer| cache.join(format!("{peer}.sefk"))),
    }
}

fn root_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(unix)]
#[test]
fn abgleich_task_keyed_walk_sends_only_what_changed_and_equals_the_full_walk() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let a = data.path().join("a");
    tree(&a);
    let root = root_of(&a);
    let backend = agent();
    let side = files(cache.path(), "a", None);
    for hash in [HashMode::None, HashMode::FullFresh] {
        let _ = std::fs::remove_file(&side.own);
        let (full, _) = read(&backend, &root, None, hash);
        assert!(full.tree.len() >= 31, "{:?}", full.tree.keys());
        assert!(full.filtered.contains_key(".versteckt") && full.filtered.contains_key("weg.tmp"));
        // First walk: nothing known, every folder sent, keys stored.
        let (first, keyed) = read(&backend, &root, Some(&side), hash);
        let (first_walk, first_items) = keyed.unwrap();
        assert_eq!(first, full);
        assert_eq!(first_walk.reused, 0);
        assert_eq!(first_walk.sent, 22, "root, leer, ten folders with inner");
        assert_eq!(
            first_items, 55,
            "15 items of the root, 1 per d*, 3 per inner"
        );
        assert!(side.own.exists(), "the side's keys are stored");
        // Unchanged: one mark for the root, no listing, no entry.
        let (second, keyed) = read(&backend, &root, Some(&side), hash);
        let (second_walk, second_items) = keyed.unwrap();
        assert_eq!(second, full);
        assert_eq!(second_items, 0, "nothing crossed the connection");
        assert_eq!(second_walk.sent, 0);
        assert_eq!(second_walk.reused, 22);
    }
    // The loop ended in the checksum mode, whose keys carry digests: the
    // walk below compares in the size-and-time mode, so its keys are stored
    // first (keys of another mode are never equal, they only cost a resend).
    let (_, keyed) = read(&backend, &root, Some(&side), HashMode::None);
    keyed.unwrap();
    // One file changes: its folders are sent, every other one is a mark.
    let changed = a.join("d3").join("inner").join("f1.txt");
    std::fs::write(&changed, b"neu und anders").unwrap();
    let (full, _) = read(&backend, &root, None, HashMode::None);
    let (keyed_side, keyed) = read(&backend, &root, Some(&side), HashMode::None);
    let (walked, items) = keyed.unwrap();
    assert_eq!(keyed_side, full);
    assert_eq!(walked.sent, 3, "root, d3 and d3/inner");
    assert_eq!(walked.reused, 19);
    assert_eq!(items, 19, "only the items of the three folders");
}

#[cfg(unix)]
#[test]
fn abgleich_task_keyed_walk_takes_what_the_other_side_stored() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let a = data.path().join("a");
    let b = data.path().join("b");
    tree(&a);
    tree(&b);
    let backend = agent();
    let side_a = files(cache.path(), "a", None);
    let side_b = files(cache.path(), "b", Some("a"));
    read(&backend, &root_of(&a), Some(&side_a), HashMode::None);
    // B equals A and has never been read with keys: A's keys answer it.
    let (full_b, _) = read(&backend, &root_of(&b), None, HashMode::None);
    let (keyed_b, keyed) = read(&backend, &root_of(&b), Some(&side_b), HashMode::None);
    let (walked, items) = keyed.unwrap();
    assert_eq!(keyed_b, full_b);
    assert_eq!(items, 0);
    assert_eq!(walked.sent, 0);
    assert!(side_b.own.exists(), "B stores its keys too");
    // The plan of both sides with a basis is that of the full walks.
    let (full_a, _) = read(&backend, &root_of(&a), None, HashMode::None);
    let mut base = Baseline::new();
    let sig = full_a.tree["a.txt"];
    let older = Sig {
        mtime_ms: sig.mtime_ms - 5_000,
        ..sig
    };
    base.insert("a.txt".into(), (Some(older), Some(sig)));
    base.insert("gone.txt".into(), (Some(sig), Some(sig)));
    let opts = BisyncOptions::default();
    let keyed_a = read(&backend, &root_of(&a), Some(&side_a), HashMode::None).0;
    let planned = super::super::core::plan(&keyed_a.tree, &keyed_b.tree, &base, opts);
    let expected = super::super::core::plan(&full_a.tree, &full_b.tree, &base, opts);
    assert_eq!(format!("{planned:?}"), format!("{expected:?}"));
}

#[cfg(unix)]
#[test]
fn abgleich_task_keyed_walk_ignores_a_damaged_key_file() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let a = data.path().join("a");
    tree(&a);
    let backend = agent();
    let side = files(cache.path(), "a", None);
    std::fs::write(&side.own, b"SEFK\x01\x00\x00\x00\x05kaputt").unwrap();
    let (full, _) = read(&backend, &root_of(&a), None, HashMode::None);
    let (keyed, walked) = read(&backend, &root_of(&a), Some(&side), HashMode::None);
    assert_eq!(keyed, full);
    assert_eq!(walked.unwrap().0.reused, 0);
    let stored = super::super::folder_key_file::load(&side.own);
    assert_eq!(stored.len(), 22, "replaced by this walk's keys");
}

struct Listed(Mutex<Vec<String>>);

impl super::super::early_copy::FolderListed for Listed {
    fn listed(&self, dir_rel: &str, _: &super::super::snapshot_dir::Listed) {
        self.0.lock().unwrap().push(dir_rel.to_string());
    }
}

fn folder(rel: &str) -> crate::vfs::HashWalkItem {
    crate::vfs::HashWalkItem::Entry(crate::vfs::HashWalkEntry {
        rel: rel.into(),
        is_dir: true,
        size: 0,
        mtime_ms: 0,
        digest: None,
        meta: Some(crate::vfs::HashWalkMeta::default()),
    })
}

/// W8: with run marks an empty folder is checked when the next mark comes,
/// a folder that could not be opened when its parent's run ends, both
/// before the walk is over.
#[test]
fn abgleich_task_run_marks_check_empty_and_unopened_folders_at_once() {
    let backend = agent();
    let cancel = AtomicBool::new(false);
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let dirs = Mutex::new(DirSet::new());
    let seen = Listed(Mutex::new(Vec::new()));
    let ctx = WalkContext {
        be: &backend,
        root: "/r",
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
        listed: Some(&seen),
        host_listed: true,
        local: None,
        key_files: None,
        pair: None,
        local_keys: None,
    };
    let mut stream = super::super::snapshot_host_stream::Stream::new(&ctx);
    stream.run_mark("").unwrap();
    for item in [folder("leer"), folder("zu"), folder("x")] {
        stream.item(item).unwrap();
    }
    stream
        .item(crate::vfs::HashWalkItem::Omitted(crate::vfs::VfsOmission {
            rel: "zu".into(),
            reason: crate::vfs::OmissionReason::Unreadable,
            detail: "Zugriff verweigert".into(),
        }))
        .unwrap();
    stream.run_mark("leer").unwrap();
    let unopened = omissions
        .lock()
        .unwrap()
        .paths()
        .any(|(path, kind, _)| path == "zu" && kind == OmissionKind::Unreadable);
    assert!(unopened, "checked when the root's run ended");
    stream.run_mark("x").unwrap();
    assert_eq!(
        *seen.0.lock().unwrap(),
        ["", "leer"],
        "the empty run is checked"
    );
    let tree = stream.finish_tree().unwrap();
    assert!(tree.is_empty());
    assert_eq!(*seen.0.lock().unwrap(), ["", "leer", "x"]);
    drop(ctx);
    let dirs: Vec<String> = dirs.into_inner().unwrap().into_iter().collect();
    assert_eq!(dirs, ["leer", "x", "zu"]);
}

#[test]
fn abgleich_task_stored_keys_are_offered_only_with_their_whole_subtree() {
    use super::super::folder_key_cache::{FolderCache, Part, Record};
    let leaf = Record {
        parts: vec![Part::Failed {
            reason: crate::vfs::OmissionReason::Unreadable,
            detail: String::new(),
        }],
        children: Vec::new(),
    };
    let missing = [9u8; 32];
    let parent = Record {
        parts: Vec::new(),
        children: vec![("fehlt".into(), missing)],
    };
    let mut cache = FolderCache::default();
    cache.insert(leaf.key(), leaf.clone());
    cache.insert(parent.key(), parent);
    let known = FolderCache::known(&[&cache]);
    assert_eq!(known, [crate::agent_proto::known_prefix(&leaf.key())]);
    // The file keeps both records; the incomplete one is still not offered.
    let bytes = super::super::folder_key_file::encode_for_test(&cache);
    let read = super::super::folder_key_file::parse_from(&bytes[..]).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(FolderCache::known(&[&read]), known);
}
