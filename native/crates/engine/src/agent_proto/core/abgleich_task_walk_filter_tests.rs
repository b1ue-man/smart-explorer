//! The SSH agent's walks with a scope (`walk-filter-v1`, `walk_filter.rs`):
//! a file the job's filter leaves out comes as an entry without a digest, a
//! folder it leaves out as an entry without its content and without a key;
//! everything else is as without the filter. A fresh walk never answers
//! from the digest cache. Folder keys of a filtered walk are equal exactly
//! when the filtered folders are, and equal on the agent and on a local
//! side with the same filter.
use super::ext_ops::handle_walk_hashed3;
use super::session::Sink;
use super::walk_keyed::{handle_walk_keyed, local_keyed_reading, WireItem};
use super::{digest, read_frame, Frame, LocalReads, WalkFilterSpec, WalkScope, WireHashEntry};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn sink(&self) -> Sink {
        Arc::new(Mutex::new(Box::new(self.clone())))
    }

    fn frames(&self) -> Vec<Frame> {
        let mut cursor = io::Cursor::new(self.0.lock().unwrap().clone());
        let mut frames = Vec::new();
        while let Some((_, frame)) = read_frame(&mut cursor).unwrap() {
            frames.push(frame);
        }
        frames
    }
}

fn write_old(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
}

/// A tree with a kept file, a `*.tmp` file, a hidden file, a file above the
/// size bound, an ignored folder (`cache/`), a hidden folder and a subfolder.
fn tree(root: &Path, cached: &[u8]) {
    write_old(&root.join("keep.txt"), b"abc");
    write_old(&root.join("skip.tmp"), b"tmpdata");
    write_old(&root.join(".hidden"), b"h");
    write_old(&root.join("big.bin"), &[7u8; 101]);
    write_old(&root.join("cache").join("inner.txt"), cached);
    write_old(&root.join(".git").join("x"), b"x");
    write_old(&root.join("sub").join("keep2.txt"), b"abc");
}

fn regex(glob: &str) -> String {
    globset::Glob::new(glob).unwrap().regex().to_string()
}

fn filter(skip_hidden: bool) -> WalkFilterSpec {
    WalkFilterSpec {
        skip_hidden,
        patterns: vec![regex("**/*.tmp"), regex("cache/")],
        backslash_separates: std::path::is_separator('\\'),
        max_size: 100,
        ..WalkFilterSpec::default()
    }
}

fn root_text(root: &Path) -> String {
    root.to_str().expect("utf-8 temp path").to_string()
}

/// Every entry of one `WalkHashed3` with attributes and MD5, by path.
fn walk(root: &Path, scope: &WalkScope) -> BTreeMap<String, WireHashEntry> {
    let captured = Captured::default();
    let request = (digest::MD5, 0, true, scope);
    let cancel = AtomicBool::new(false);
    handle_walk_hashed3(&captured.sink(), 4, &root_text(root), request, &cancel).unwrap();
    let mut entries = BTreeMap::new();
    for frame in captured.frames() {
        match frame {
            Frame::HashPart { entries: part, .. } => {
                for entry in part {
                    entries.insert(entry.rel.clone(), entry);
                }
            }
            Frame::End => {}
            other => panic!("unexpected frame {other:?}"),
        }
    }
    entries
}

#[test]
fn abgleich_task_agent_walk_hashes_and_sends_nothing_the_filter_leaves_out() {
    let data = tempfile::tempdir().unwrap();
    tree(data.path(), b"inner");
    let filtered = WalkScope {
        fresh: false,
        filter: Some(filter(true)),
    };
    let entries = walk(data.path(), &filtered);
    let digest_of = |rel: &str| entries[rel].digest.clone();
    assert_eq!(digest_of("keep.txt").as_deref(), Some(ABC_MD5));
    assert_eq!(digest_of("sub/keep2.txt").as_deref(), Some(ABC_MD5));
    // Left out: still an entry (the planner keeps it as a filtered file),
    // never hashed.
    for rel in ["skip.tmp", ".hidden", "big.bin"] {
        assert!(entries.contains_key(rel), "{rel} is still sent");
        assert_eq!(digest_of(rel), None, "{rel} is not hashed");
    }
    // A folder left out is an entry; nothing below it is read or sent.
    for (folder, inner) in [("cache", "cache/inner.txt"), (".git", ".git/x")] {
        assert!(entries[folder].is_dir, "{folder} is still sent");
        assert!(!entries.contains_key(inner), "{inner} is not sent");
    }
    // Without a filter the walk is the former one.
    let plain = walk(data.path(), &WalkScope::default());
    assert!(plain["skip.tmp"].digest.is_some());
    assert!(plain.contains_key("cache/inner.txt"));
    assert!(plain.contains_key(".git/x"));
    assert_eq!(
        plain
            .keys()
            .filter(|rel| !rel.starts_with("cache/") && !rel.starts_with(".git/"))
            .count(),
        entries.len(),
        "the filter only leaves out what is below the folders it leaves out"
    );
}

#[test]
fn abgleich_task_agent_fresh_walk_never_answers_from_the_digest_cache() {
    let _turn = super::hash_cache::test_turn();
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("keep.txt");
    write_old(&path, b"abc");
    // The status-change time must lie outside the racy slack of the cache.
    std::thread::sleep(Duration::from_millis(2_100));
    let wrong = "0123456789abcdef0123456789abcdef".to_string();
    let stamp = || super::path_stamp(&path);
    // A wrong digest stored for the unchanged file (as a stale cache would).
    assert!(
        stamp().is_some(),
        "no durable file stamp on this test file system: the test proves nothing here"
    );
    let _ = super::hash_cache::digest(stamp(), digest::MD5, || Ok(wrong.clone()), stamp);
    // The walk without `fresh` answers from the cache: the cache really
    // holds the wrong digest, so the fresh walk below has something to skip.
    let cached = walk(data.path(), &WalkScope::default())["keep.txt"]
        .digest
        .clone();
    assert_eq!(
        cached.as_deref(),
        Some(wrong.as_str()),
        "the cache did not hold the stored digest: the test proves nothing"
    );
    let fresh = WalkScope {
        fresh: true,
        filter: None,
    };
    let read = walk(data.path(), &fresh)["keep.txt"].digest.clone();
    assert_eq!(
        read.as_deref(),
        Some(ABC_MD5),
        "the checksum mode reads the file"
    );
}

/// Every folder key of the local keyed walk with `filter`.
fn local_keys(root: &Path, algorithm: u8, filter: Option<&WalkFilterSpec>) -> super::LocalKeyed {
    let compiled = filter.map(|filter| filter.compile().unwrap());
    let scope = (LocalReads::default(), compiled.as_ref());
    local_keyed_reading(&root_text(root), algorithm, scope, &AtomicBool::new(false)).unwrap()
}

fn key_of(collected: &super::LocalKeyed, rel: &str) -> Option<[u8; 32]> {
    collected
        .keys
        .iter()
        .find(|(folder, _)| folder == rel)
        .map(|(_, key)| *key)
}

#[test]
fn abgleich_task_filtered_folder_keys_mean_equal_filtered_folders() {
    let data = tempfile::tempdir().unwrap();
    let (a, b) = (data.path().join("a"), data.path().join("b"));
    // The two trees differ only below the folder the filter leaves out.
    tree(&a, b"one");
    tree(&b, b"two two");
    let spec = filter(false);
    for algorithm in [digest::NONE, digest::MD5] {
        let (fa, fb) = (
            local_keys(&a, algorithm, Some(&spec)),
            local_keys(&b, algorithm, Some(&spec)),
        );
        assert_eq!(key_of(&fa, ""), key_of(&fb, ""), "equal filtered trees");
        assert_eq!(key_of(&fa, "cache"), None, "a folder left out has no key");
        assert!(fa.runs.iter().all(|(rel, _)| rel != "cache"));
        let (pa, pb) = (
            local_keys(&a, algorithm, None),
            local_keys(&b, algorithm, None),
        );
        assert_ne!(key_of(&pa, ""), key_of(&pb, ""), "unequal unfiltered trees");
        assert!(key_of(&pa, "cache").is_some());
        // A stored key of the unfiltered walk never stands for the filtered
        // folder, nor the other way round.
        assert_ne!(key_of(&pa, ""), key_of(&fa, ""));
        if algorithm == digest::MD5 {
            let root_run = &fa.runs.iter().find(|(rel, _)| rel.is_empty()).unwrap().1;
            for item in root_run {
                if let WireItem::Entry(entry) = item {
                    let left_out = entry.rel == "skip.tmp" || entry.rel == "big.bin";
                    let file = !entry.is_dir && entry.meta.is_some_and(|m| !m.is_symlink);
                    if file {
                        assert_eq!(entry.digest.is_none(), left_out, "{}", entry.rel);
                    }
                }
            }
        }
    }
}

/// The agent and a local side form the same keys with the same filter
/// (hidden names follow the same rule only off Windows).
#[cfg(unix)]
#[test]
fn abgleich_task_agent_and_local_side_form_equal_filtered_keys() {
    let data = tempfile::tempdir().unwrap();
    tree(data.path(), b"inner");
    let spec = filter(true);
    let local = local_keys(data.path(), digest::MD5, Some(&spec));
    let captured = Captured::default();
    let scope = WalkScope {
        fresh: true,
        filter: Some(spec),
    };
    handle_walk_keyed(
        &captured.sink(),
        5,
        &root_text(data.path()),
        (digest::MD5, 0, &[][..], true),
        &scope,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut agent = BTreeMap::new();
    for frame in captured.frames() {
        if let Frame::FolderKey { rel, key, .. } = frame {
            agent.insert(rel, key);
        }
    }
    let local: BTreeMap<String, [u8; 32]> = local.keys.into_iter().collect();
    assert_eq!(agent, local);
    assert!(!agent.contains_key("cache") && !agent.contains_key(".git"));
}
