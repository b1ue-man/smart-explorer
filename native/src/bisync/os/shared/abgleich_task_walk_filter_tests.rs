//! A sync side read by the agent next to its data with the job's filter
//! (`walk-filter-v1`) gives the snapshot of the walk without it: the same
//! files, filtered files, folders and omissions, in both hash modes, through
//! the stream and through the walk with folder keys; the filtered walk
//! sends less (nothing below a folder it leaves out), and its folder keys
//! are stored and answer the next walk although such a folder has none.
//! Unix only (the module is declared with `cfg(all(test, unix))`).
#![cfg(unix)]
use super::super::omissions::{OmissionKind, SyncOmissions};
use super::super::snapshot::WalkFilter;
use super::super::snapshot_dir::WalkContext;
use super::super::snapshot_hash::HashMode;
use super::super::snapshot_keyed::{walk as keyed_walk, KeyFiles};
use super::super::snapshot_types::DirSet;
use super::super::sync_overload::Progress;
use super::super::types::{BisyncOptions, Tree};
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

#[derive(Debug, PartialEq)]
struct Side {
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

fn tree(root: &Path) {
    write_old(&root.join("keep.txt"), b"abc");
    write_old(&root.join("skip.tmp"), b"tmp");
    write_old(&root.join(".hidden"), b"h");
    write_old(&root.join("big.bin"), &[1u8; 200]);
    for index in 0..5 {
        write_old(&root.join("cache").join(format!("c{index}.txt")), b"cached");
    }
    write_old(&root.join(".git").join("objects").join("o"), b"o");
    write_old(&root.join("sub").join("x.tmp"), b"x");
    write_old(&root.join("sub").join("keep2.txt"), b"keep");
}

/// What one walk of `root` gave, how it went with keys, and how many items
/// crossed the connection.
fn read(
    backend: &AgentBackend,
    root: &str,
    filter: &WalkFilter<'_>,
    hash: HashMode,
    keyed: Option<(&KeyFiles, bool)>,
) -> (Side, Option<(u64, u64)>, u64) {
    let cancel = AtomicBool::new(false);
    let omissions = Mutex::new(SyncOmissions::new(false));
    let filtered = Mutex::new(Tree::new());
    let dirs = Mutex::new(DirSet::new());
    let ctx = WalkContext {
        be: backend,
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
    let mut items = 0u64;
    let (tree, counts) = match keyed {
        None => (
            super::walk_snapshot_via_agent(&ctx)
                .unwrap()
                .expect("the agent walks next to the data"),
            None,
        ),
        Some((files, with_filter)) => {
            let request = HashWalkRequest {
                algorithm: (hash == HashMode::FullFresh)
                    .then_some(crate::analytics::HashAlgorithm::Md5),
                min_bytes: 0,
                meta: true,
                fresh: hash == HashMode::FullFresh,
                filter: if with_filter {
                    filter.host_spec()
                } else {
                    None
                },
            };
            let walked = keyed_walk(&ctx, files, |take, known| {
                let (tx, rx) = crossbeam_channel::unbounded();
                let marks = vfs::KeyedWalk::Marks { known };
                let ran = vfs::hash_walk_keyed(backend, root, request, marks, tx, &cancel)?;
                for item in rx.try_iter() {
                    if matches!(item, KeyedWalkItem::Item(_)) {
                        items += 1;
                    }
                    take(item)?;
                }
                Ok(ran)
            })
            .unwrap()
            .expect("the agent offers folder keys");
            (walked.tree, Some((walked.sent, walked.reused)))
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
    (side, counts, items)
}

#[test]
fn abgleich_task_filtered_host_walk_gives_the_snapshot_of_the_unfiltered_one() {
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let a = data.path().join("a");
    tree(&a);
    let root = a.to_string_lossy().into_owned();
    let mut job = crate::syncjobs::SyncJob::new("filter".into(), "/a".into(), "/b".into());
    job.ignore = vec!["**/*.tmp".into(), "cache/".into()];
    let ignore = job.checked_ignore().unwrap();
    assert_eq!(
        ignore.host.len(),
        2,
        "both patterns are applied next to the data"
    );
    let filter = WalkFilter {
        include_hidden: false,
        ignore: &ignore.globs,
        host_ignore: &ignore.host,
        max_size: 100,
        ..WalkFilter::basic(false, &ignore.globs)
    };
    let spec = filter.host_spec().expect("the filter leaves something out");
    assert!(spec.skip_hidden && spec.max_size == 100 && spec.patterns.len() == 2);
    let backend = agent();
    for hash in [HashMode::None, HashMode::FullFresh] {
        let plain_keys = KeyFiles {
            own: cache.path().join(format!("plain-{hash:?}.sefk")),
            peer: None,
        };
        let filtered_keys = KeyFiles {
            own: cache.path().join(format!("filtered-{hash:?}.sefk")),
            peer: None,
        };
        let (plain, _, plain_items) =
            read(&backend, &root, &filter, hash, Some((&plain_keys, false)));
        assert!(plain.filtered.contains_key("skip.tmp"));
        assert!(plain.filtered.contains_key("big.bin"));
        assert!(plain.filtered.contains_key(".hidden"));
        assert!(plain.tree.contains_key("keep.txt"));
        // Through the stream and through the keyed walk with the filter.
        let (streamed, _, _) = read(&backend, &root, &filter, hash, None);
        assert_eq!(streamed, plain, "{hash:?}: the stream");
        let (keyed, counts, items) =
            read(&backend, &root, &filter, hash, Some((&filtered_keys, true)));
        assert_eq!(keyed, plain, "{hash:?}: the keyed walk");
        assert!(
            items + 7 <= plain_items,
            "{hash:?}: nothing below cache/ and .git/ crossed ({items} of {plain_items})"
        );
        assert_eq!(counts.map(|(_, reused)| reused), Some(0));
        assert!(filtered_keys.own.exists(), "the filtered keys are stored");
        // The stored filtered keys answer the next filtered walk.
        let (again, counts, items) =
            read(&backend, &root, &filter, hash, Some((&filtered_keys, true)));
        assert_eq!(again, plain, "{hash:?}: answered by the stored keys");
        assert_eq!(items, 0, "{hash:?}: nothing crossed the connection");
        assert_eq!(counts.map(|(sent, _)| sent), Some(0));
        // The unfiltered keys never answer the filtered walk: other keys.
        let crossed = KeyFiles {
            own: filtered_keys.own.clone(),
            peer: Some(plain_keys.own.clone()),
        };
        std::fs::remove_file(&crossed.own).unwrap();
        let (mixed, counts, _) = read(&backend, &root, &filter, hash, Some((&crossed, true)));
        assert_eq!(mixed, plain, "{hash:?}: the other side's unfiltered keys");
        assert!(counts.is_some_and(|(sent, _)| sent > 0), "the root differs");
    }
}
