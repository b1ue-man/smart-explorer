//! A local side's folder keys come from its one snapshot walk
//! (`snapshot_local_keys.rs`): they equal, folder by folder, the keys the
//! walk next to the data forms for the same tree with the same filter (the
//! agent's walk, run here in this process), in both hash modes; the walk's
//! snapshot (files, filtered files, folders, omissions) is the one of the
//! walk without keys; and nothing lists the tree a second time, except a
//! folder the job's filter leaves out although the other side's walk
//! enters it, which that walk reads here once (and a file it hashes).
use super::super::omissions::{OmissionKind, SyncOmissions};
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::WalkContext;
use super::super::snapshot_hash::HashMode;
use super::super::snapshot_policy::LocalFacts;
use super::super::snapshot_types::DirSet;
use super::super::snapshot_walk::walk_tree;
use super::super::sync_overload::Progress;
use super::super::types::{BisyncOptions, Tree};
use super::LocalKeys;
use crate::agent_proto::{digest, FolderKey, LocalReads, WalkFilterSpec};
use crate::vfs::LocalBackend;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

#[derive(Debug, PartialEq)]
struct Seen {
    tree: Tree,
    filtered: Tree,
    dirs: Vec<String>,
    omissions: Vec<(String, OmissionKind)>,
}

fn write_old(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
}

/// Files, a hidden file and folder, a filtered file and folder, an empty
/// folder and nested folders.
fn tree(root: &Path) {
    write_old(&root.join("keep.txt"), b"abc");
    write_old(&root.join("skip.tmp"), b"tmp");
    write_old(&root.join(".hidden"), b"h");
    write_old(&root.join(".hdir").join("x.txt"), b"x");
    write_old(&root.join("sub").join("a.txt"), b"aa");
    write_old(&root.join("sub").join("inner").join("b.txt"), b"bbb");
    write_old(&root.join("cache").join("inner.txt"), b"cache");
    write_old(&root.join("cache").join("deep").join("more.txt"), b"more");
    std::fs::create_dir_all(root.join("leer")).unwrap();
}

fn root_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The job's patterns as globset forms them, and their regular expressions.
fn job(globs: &[&str]) -> (globset::GlobSet, Vec<String>) {
    let mut builder = globset::GlobSetBuilder::new();
    let mut regexes = Vec::new();
    for glob in globs {
        let glob = globset::Glob::new(glob).unwrap();
        regexes.push(glob.regex().to_string());
        builder.add(glob);
    }
    (builder.build().unwrap(), regexes)
}

/// The snapshot walk of the local side `root` (with `keys`: forming them).
fn walk(root: &str, filter: &WalkFilter, hash: HashMode, keys: Option<&LocalKeys>) -> Seen {
    let be = LocalBackend::new(root);
    let cancel = AtomicBool::new(false);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let filtered = Mutex::new(Tree::new());
    let dirs = Mutex::new(DirSet::new());
    let ctx = WalkContext {
        be: &be,
        root,
        cancel: &cancel,
        filter,
        hash,
        prev: None,
        allow_duplicate_files: false,
        omissions: Some(&omissions),
        duplicates: None,
        filtered: Some(&filtered),
        dirs: Some(&dirs),
        opts: BisyncOptions {
            cross_mounts: true,
            ..Default::default()
        },
        limits: super::super::SyncLimits::for_memory(None),
        nodes: AtomicU64::new(0),
        text_bytes: AtomicU64::new(0),
        reads: None,
        progress: Progress::default(),
        log: None,
        listed: None,
        host_listed: false,
        local: Some(LocalFacts::new(&be, root)),
        key_files: None,
        pair: None,
        local_keys: keys,
    };
    let tree = walk_tree(&ctx, None, 4).unwrap();
    drop(ctx);
    Seen {
        tree,
        filtered: filtered.into_inner().unwrap(),
        dirs: dirs.into_inner().unwrap().into_iter().collect(),
        omissions: omissions
            .into_inner()
            .unwrap()
            .paths()
            .map(|(path, kind, _)| (path.to_string(), kind))
            .collect(),
    }
}

/// Every folder key of the walk next to the data with `spec`, in this
/// process (as the other side's agent forms them for the same tree).
fn agent_keys(root: &str, md5: bool, spec: Option<&WalkFilterSpec>) -> HashMap<String, FolderKey> {
    let filter = spec.map(|spec| spec.compile().unwrap());
    let reads = LocalReads {
        fresh: md5,
        run: None,
    };
    let algorithm = if md5 { digest::MD5 } else { digest::NONE };
    let cancel = AtomicBool::new(false);
    crate::agent_proto::local_keyed_reading(root, algorithm, (reads, filter.as_ref()), &cancel)
        .unwrap()
        .keys
        .into_iter()
        .collect()
}

/// Keys of the one walk, the folders read below, and its snapshot.
fn one_walk(
    root: &str,
    filter: &WalkFilter,
    hash: HashMode,
) -> (HashMap<String, FolderKey>, Vec<String>, Seen) {
    let md5 = hash == HashMode::FullFresh;
    let keys = LocalKeys::new(root, md5, filter.host_spec());
    let seen = walk(root, filter, hash, Some(&keys));
    let below = keys.read_below_folders();
    let side = keys.finish();
    // Every key names its record: the runs replay what was listed.
    for (rel, key) in &side.keys {
        assert!(side.records.get(key).is_some(), "{rel} has its record");
    }
    (side.keys, below, seen)
}

#[test]
fn abgleich_task_local_side_keys_come_from_its_one_walk_and_equal_the_agents() {
    let _turn = crate::agent_proto::hash_cache::test_turn();
    let data = tempfile::tempdir().unwrap();
    tree(data.path());
    let root = root_of(data.path());
    let (globs, regexes) = job(&["**/*.tmp", "cache"]);
    let host = crate::agent_proto::host_patterns(regexes);
    assert_eq!(host.len(), 2, "the other side's walk takes both patterns");
    let filter = WalkFilter {
        host_ignore: &host,
        ..WalkFilter::basic(false, &globs)
    };
    let spec = filter.host_spec().expect("the job filters");
    for hash in [HashMode::None, HashMode::FullFresh] {
        let (keys, below, seen) = one_walk(&root, &filter, hash);
        let md5 = hash == HashMode::FullFresh;
        assert_eq!(
            keys,
            agent_keys(&root, md5, Some(&*spec)),
            "equal folders, equal keys ({hash:?})"
        );
        // Folders the filter leaves out have no key on either side (a dot
        // name is hidden only outside Windows, where the attribute counts).
        assert!(!keys.contains_key("cache"));
        assert_eq!(keys.contains_key(".hdir"), cfg!(windows));
        for rel in ["", "sub", "sub/inner", "leer"] {
            assert!(keys.contains_key(rel), "{rel}");
        }
        assert!(below.is_empty(), "the tree is listed once: {below:?}");
        assert_eq!(
            seen,
            walk(&root, &filter, hash, None),
            "the snapshot is the one of the walk without keys ({hash:?})"
        );
    }
}

#[test]
fn abgleich_task_local_side_reads_below_only_what_the_other_side_enters() {
    let _turn = crate::agent_proto::hash_cache::test_turn();
    let data = tempfile::tempdir().unwrap();
    tree(data.path());
    write_old(&data.path().join("note.log"), b"log");
    let root = root_of(data.path());
    let (globs, regexes) = job(&["**/*.tmp", "cache", "**/*.log"]);
    // The other side's walk applies only the first pattern (the others stand
    // for patterns it cannot evaluate): it enters `cache`, hashes `note.log`.
    let host = vec![regexes[0].clone()];
    let filter = WalkFilter {
        host_ignore: &host,
        ..WalkFilter::basic(true, &globs)
    };
    let spec = filter.host_spec().expect("the job filters");
    for hash in [HashMode::None, HashMode::FullFresh] {
        let md5 = hash == HashMode::FullFresh;
        let (keys, below, seen) = one_walk(&root, &filter, hash);
        assert_eq!(below, vec!["cache".to_string()], "{hash:?}");
        assert_eq!(keys, agent_keys(&root, md5, Some(&*spec)), "{hash:?}");
        assert!(keys.contains_key("cache") && keys.contains_key("cache/deep"));
        // The snapshot still leaves `cache` and `note.log` out.
        assert_eq!(seen, walk(&root, &filter, hash, None), "{hash:?}");
        assert!(!seen.tree.keys().any(|rel| rel.starts_with("cache")));
        assert!(!seen.tree.contains_key("note.log"));
    }
}

#[test]
fn abgleich_task_local_side_keys_change_with_the_content_of_a_folder() {
    let data = tempfile::tempdir().unwrap();
    tree(data.path());
    let root = root_of(data.path());
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let (before, _, _) = one_walk(&root, &filter, HashMode::None);
    write_old(&data.path().join("sub").join("inner").join("b.txt"), b"b");
    let (after, _, _) = one_walk(&root, &filter, HashMode::None);
    for rel in ["", "sub", "sub/inner"] {
        assert_ne!(before[rel], after[rel], "{rel} changed");
    }
    for rel in ["leer", "cache", "cache/deep", ".hdir"] {
        assert_eq!(before[rel], after[rel], "{rel} is unchanged");
    }
}

#[cfg(unix)]
#[test]
fn abgleich_task_local_side_keys_with_links_equal_the_agents() {
    let data = tempfile::tempdir().unwrap();
    tree(data.path());
    std::os::unix::fs::symlink(data.path().join("sub"), data.path().join("to-sub")).unwrap();
    std::os::unix::fs::symlink("keep.txt", data.path().join("to-keep")).unwrap();
    let root = root_of(data.path());
    let globs = globset::GlobSet::empty();
    let filter = WalkFilter::basic(true, &globs);
    let (keys, below, seen) = one_walk(&root, &filter, HashMode::None);
    assert_eq!(keys, agent_keys(&root, false, None));
    assert!(below.is_empty());
    // A link is a protected omission, never followed.
    assert!(seen
        .omissions
        .contains(&("to-sub".to_string(), OmissionKind::Link)));
    assert!(!keys.contains_key("to-sub"));
    assert_eq!(seen, walk(&root, &filter, HashMode::None, None));
}
