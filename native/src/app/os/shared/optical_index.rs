//! Immutable optical index reuse. All probes, certificate I/O, hashing and
//! persistence run on this worker; the UI supplies an immutable old snapshot.
use crate::folder_index::{FolderIndex, IndexMsg};
use crate::source_media::Source;
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone, Serialize, Deserialize)]
struct Proof {
    device: String,
    generation: String,
    root_identity: String,
    paths_hash: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Certificate {
    version: u32,
    roots: BTreeMap<String, Proof>,
}

/// `allow_reuse=false` is an explicit full rebuild. A legacy index without a
/// valid private certificate always traverses its optical roots once.
pub(crate) fn build_async(
    roots: Vec<PathBuf>,
    persist: PathBuf,
    previous: FolderIndex,
    tx: Sender<IndexMsg>,
    cancel: Arc<AtomicBool>,
    allow_reuse: bool,
) -> io::Result<()> {
    spawn(roots, persist, previous, tx, cancel, allow_reuse, false)
}

/// Startup verification preserves every non-optical index entry. Only actual
/// optical roots without a current matching proof are enumerated again.
pub(crate) fn verify_loaded_async(
    roots: Vec<PathBuf>,
    persist: PathBuf,
    previous: FolderIndex,
    tx: Sender<IndexMsg>,
    cancel: Arc<AtomicBool>,
) -> io::Result<()> {
    spawn(roots, persist, previous, tx, cancel, true, true)
}

fn spawn(
    roots: Vec<PathBuf>,
    persist: PathBuf,
    previous: FolderIndex,
    tx: Sender<IndexMsg>,
    cancel: Arc<AtomicBool>,
    allow_reuse: bool,
    optical_only: bool,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("optical-index-plan".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if optical_only {
                    build_scoped(roots, &persist, &previous, &tx, &cancel, allow_reuse, true)
                } else {
                    build(roots, &persist, &previous, &tx, &cancel, allow_reuse)
                }
            }));
            let terminal = match result {
                Ok(Ok(index)) => IndexMsg::Complete(index),
                Ok(Err(_)) if cancel.load(Ordering::Acquire) => IndexMsg::Canceled,
                Ok(Err(error)) => IndexMsg::Failed(error.to_string()),
                Err(_) => IndexMsg::Failed("optical index worker panicked".into()),
            };
            let _ = tx.send(terminal);
        })
        .map(|_| ())
}

fn build(
    roots: Vec<PathBuf>,
    persist: &Path,
    previous: &FolderIndex,
    tx: &Sender<IndexMsg>,
    cancel: &Arc<AtomicBool>,
    allow_reuse: bool,
) -> io::Result<FolderIndex> {
    build_scoped(roots, persist, previous, tx, cancel, allow_reuse, false)
}

fn build_scoped(
    roots: Vec<PathBuf>,
    persist: &Path,
    previous: &FolderIndex,
    tx: &Sender<IndexMsg>,
    cancel: &Arc<AtomicBool>,
    allow_reuse: bool,
    optical_only: bool,
) -> io::Result<FolderIndex> {
    if roots.is_empty() {
        return Err(io::Error::other("folder-index roots are empty"));
    }
    let certificate_path = persist.with_extension("optical-index.json");
    let stored = crate::support_dirs::read_private_text(&certificate_path, 4 * 1024 * 1024)
        .ok()
        .and_then(|text| serde_json::from_str::<Certificate>(&text).ok())
        .filter(|proof| proof.version == 2)
        .unwrap_or_default();
    let mut observed = Vec::<(PathBuf, Source, String)>::new();
    let mut excluded = Vec::new();
    let mut scan = Vec::new();
    let mut merged = if optical_only {
        previous.clone()
    } else {
        FolderIndex::new()
    };
    let mut optical_found = false;
    for root in &roots {
        check(cancel)?;
        let source = crate::source_media::probe(root)?;
        if source.is_some() {
            optical_found = true;
            if optical_only {
                let key = root_key(root)?;
                let stale: Vec<_> = merged
                    .iter()
                    .filter(|path| beneath(path, &key))
                    .cloned()
                    .collect();
                for path in stale {
                    merged.remove(&path);
                }
            }
        } else if optical_only {
            continue;
        }
        let reusable = if let Some(source) = source.as_ref().filter(|source| source.read_only) {
            let key = root_key(root)?;
            let identity = root_identity(root);
            let valid = identity.is_some()
                && allow_reuse
                && stored.roots.get(&key).is_some_and(|proof| {
                    proof.device == source.device
                        && proof.generation == source.generation
                        && Some(&proof.root_identity) == identity.as_ref()
                        && proof.paths_hash == paths_hash(previous, &key)
                });
            if let Some(identity) = identity {
                observed.push((root.clone(), source.clone(), identity));
            }
            valid
        } else {
            false
        };
        if reusable {
            let key = root_key(root)?;
            for path in previous.iter().filter(|path| beneath(path, &key)) {
                merged.try_insert(path.clone())?;
            }
            excluded.push(root.clone());
        } else {
            scan.push(root.clone());
        }
    }
    if optical_only && !optical_found {
        return Ok(merged);
    }
    if !scan.is_empty() {
        let parent = persist
            .parent()
            .ok_or_else(|| io::Error::other("folder index requires a parent"))?;
        std::fs::create_dir_all(parent)?;
        let stage = tempfile::Builder::new()
            .prefix(".optical-index-")
            .tempdir_in(parent)?;
        let (worker_tx, worker_rx) = crossbeam_channel::unbounded();
        FolderIndex::build_async_excluding(
            scan,
            stage.path().join("index.bin"),
            worker_tx,
            cancel.clone(),
            excluded,
        )?;
        loop {
            match worker_rx
                .recv()
                .map_err(|_| io::Error::other("folder-index worker closed without result"))?
            {
                IndexMsg::Progress { count, current } => {
                    if tx
                        .send(IndexMsg::Progress {
                            count: count.saturating_add(merged.len() as u64),
                            current,
                        })
                        .is_err()
                    {
                        cancel.store(true, Ordering::Release);
                    }
                }
                IndexMsg::Complete(index) => {
                    for path in index.iter() {
                        merged.try_insert(path.clone())?;
                    }
                    break;
                }
                IndexMsg::Canceled => {
                    cancel.store(true, Ordering::Release);
                    return Err(io::Error::other("index canceled"));
                }
                IndexMsg::Failed(error) => return Err(io::Error::other(error)),
            }
        }
    }
    check(cancel)?;
    let mut certificate = Certificate {
        version: 2,
        roots: BTreeMap::new(),
    };
    for (root, expected, identity) in &observed {
        validate_root(root, expected, identity)?;
        let key = root_key(root)?;
        certificate.roots.insert(
            key.clone(),
            Proof {
                device: expected.device.clone(),
                generation: expected.generation.clone(),
                root_identity: identity.clone(),
                paths_hash: paths_hash(&merged, &key),
            },
        );
    }
    if !merged.save_with_cancel(persist, cancel)? {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "index save canceled",
        ));
    }
    check(cancel)?;
    for (root, expected, identity) in &observed {
        validate_root(root, expected, identity)?;
    }
    let bytes = serde_json::to_vec(&certificate).map_err(io::Error::other)?;
    crate::support_dirs::write_private_atomic(&certificate_path, &bytes).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("index saved, optical reuse proof could not be saved: {error}"),
        )
    })?;
    Ok(merged)
}

fn validate_root(root: &Path, expected: &Source, identity: &str) -> io::Result<()> {
    if crate::source_media::probe(root)?.as_ref() != Some(expected) {
        return Err(io::Error::other(format!(
            "optical medium changed at {}",
            root.display()
        )));
    }
    if root_identity(root).as_deref() != Some(identity) {
        return Err(io::Error::other(
            "optical index root changed while building",
        ));
    }
    Ok(())
}

fn check(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "index canceled"))
    } else {
        Ok(())
    }
}
fn root_key(root: &Path) -> io::Result<String> {
    let text = root
        .to_str()
        .ok_or_else(|| io::Error::other("index root is not Unicode"))?
        .replace('\\', "/");
    Ok(if text == "/" {
        text
    } else {
        text.trim_end_matches('/').to_string()
    })
}
fn beneath(path: &str, root: &str) -> bool {
    path == root
        || if root == "/" {
            path.starts_with('/')
        } else {
            path.strip_prefix(root)
                .is_some_and(|tail| tail.starts_with('/'))
        }
}
// Bind aliases/subdirectory remounts to the actual root object as well as
// its medium. Without a proved root identity we conservatively keep walking.
fn root_identity(path: &Path) -> Option<String> {
    crate::local_access::ConfinedDirectory::open_root(path)
        .ok()?
        .identity()
        .ok()
}
fn paths_hash(index: &FolderIndex, root: &str) -> String {
    let mut paths: Vec<_> = index.iter().filter(|path| beneath(path, root)).collect();
    paths.sort_unstable();
    let mut hash = Sha256::new();
    for path in paths {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_task_optical_index_requires_proof_reuses_and_invalidates() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("disc");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("actual")).unwrap();
        let source = Source {
            device: format!("index-test:{}", root.display()),
            generation: "boot-a:disc-a".into(),
            read_only: true,
        };
        let profile = crate::source_media::test_profiles::register(&root, source.clone());
        let persist = fixture.path().join("index.bin");
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut legacy = FolderIndex::new();
        legacy.insert(root.join("invented").to_string_lossy().replace('\\', "/"));
        let first = build(vec![root.clone()], &persist, &legacy, &tx, &cancel, true).unwrap();
        assert!(first.iter().any(|path| path.ends_with("/actual")));
        assert!(!first.iter().any(|path| path.ends_with("/invented")));
        let acquisitions = crate::source_media::test_profiles::acquisitions(&source.device);
        let second = build(vec![root.clone()], &persist, &first, &tx, &cancel, true).unwrap();
        assert_eq!(
            crate::source_media::test_profiles::acquisitions(&source.device),
            acquisitions,
            "unchanged certified read-only root was enumerated again"
        );
        let mut changed = source.clone();
        changed.generation = "boot-b:disc-a".into();
        profile.replace(changed);
        let third = build(vec![root.clone()], &persist, &second, &tx, &cancel, true).unwrap();
        assert!(
            crate::source_media::test_profiles::acquisitions(&source.device) > acquisitions,
            "a changed boot/media generation reused its old proof"
        );
        let ordinary = fixture.path().join("ordinary");
        std::fs::create_dir(&ordinary).unwrap();
        let retained = ordinary
            .join("retained-from-existing-index")
            .to_string_lossy()
            .replace('\\', "/");
        let mut loaded = third.clone();
        loaded.insert(retained.clone());
        let verified = build_scoped(
            vec![ordinary, root.clone()],
            &persist,
            &loaded,
            &tx,
            &cancel,
            true,
            true,
        )
        .unwrap();
        assert!(
            verified.iter().any(|path| path == &retained),
            "startup verification rebuilt the non-optical tree"
        );
        assert!(
            FolderIndex::load(&persist)
                .unwrap()
                .iter()
                .any(|path| path == &retained),
            "startup wrote a partial index as the full index"
        );
        let before = std::fs::read(&persist).unwrap();
        cancel.store(true, Ordering::Release);
        assert!(build(vec![root], &persist, &third, &tx, &cancel, true).is_err());
        assert_eq!(std::fs::read(&persist).unwrap(), before);
    }
}
