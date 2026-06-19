use crossbeam_channel::Sender;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::filters::should_skip_meta;
use super::model::{FolderIndex, IndexMsg};

impl FolderIndex {
    /// Build an index by walking the given roots in parallel, collecting only
    /// directories. Sends progress over `tx` and posts the final result.
    pub fn build_async(roots: Vec<PathBuf>, tx: Sender<IndexMsg>, cancel: Arc<AtomicBool>) {
        std::thread::Builder::new()
            .name("index-builder".into())
            .spawn(move || {
                let paths = Mutex::new(Vec::<String>::with_capacity(200_000));
                let counter = Arc::new(AtomicU64::new(0));
                let last_emit = Arc::new(Mutex::new(std::time::Instant::now()));

                for root in &roots {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    walk_folders(root.clone(), &paths, &counter, &cancel, &tx, &last_emit);
                }

                let collected: Vec<String> = paths.into_inner().unwrap_or_default();
                let mut set: HashSet<String> = HashSet::with_capacity(collected.len());
                for p in collected {
                    set.insert(p);
                }

                let _ = tx.send(IndexMsg::Done(FolderIndex {
                    paths: set,
                    built_at: Some(SystemTime::now()),
                }));
            })
            .ok();
    }
}

/// I/O part of the search: stat the candidates and sort by
/// (score DESC, mtime DESC). Free function on owned data so callers can run
/// it on a background thread without borrowing the index.
pub fn stat_and_rank(candidates: Vec<(String, i32)>, max: usize) -> Vec<(String, i32)> {
    let mut with_mtime: Vec<(String, i32, i64)> = candidates
        .into_par_iter()
        .map(|(p, score)| {
            let mtime = std::fs::metadata(&p)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            (p, score, mtime)
        })
        .collect();
    // Score primary (desc), mtime secondary (desc, most recent first)
    with_mtime.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
    with_mtime.truncate(max);
    with_mtime.into_iter().map(|(p, s, _)| (p, s)).collect()
}

fn walk_folders(
    root: PathBuf,
    paths: &Mutex<Vec<String>>,
    counter: &Arc<AtomicU64>,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<IndexMsg>,
    last_emit: &Arc<Mutex<std::time::Instant>>,
) {
    // First add the root itself
    if let Ok(_) = std::fs::metadata(&root) {
        let rs = root.to_string_lossy().replace('\\', "/");
        paths.lock().unwrap().push(rs);
        counter.fetch_add(1, Ordering::Relaxed);
    }
    walk_parallel(vec![root], paths, counter, cancel, tx, last_emit);
}

fn walk_parallel(
    dirs: Vec<PathBuf>,
    paths: &Mutex<Vec<String>>,
    counter: &Arc<AtomicU64>,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<IndexMsg>,
    last_emit: &Arc<Mutex<std::time::Instant>>,
) {
    if dirs.is_empty() || cancel.load(Ordering::Relaxed) {
        return;
    }
    dirs.into_par_iter().for_each(|dir| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut local_dirs: Vec<String> = Vec::with_capacity(16);
        let mut subdirs: Vec<PathBuf> = Vec::with_capacity(16);
        for er in read {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let entry = match er {
                Ok(e) => e,
                Err(_) => continue,
            };
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_dir() || meta.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // Filter out hidden / system / dotfolders / generic IDs.
            #[cfg(windows)]
            let attrs = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes()
            };
            #[cfg(not(windows))]
            let attrs: u32 = 0;
            if should_skip_meta(&name, attrs) {
                continue;
            }
            let path = entry.path();
            let s = path.to_string_lossy().replace('\\', "/");
            local_dirs.push(s);
            subdirs.push(path);
        }
        if !local_dirs.is_empty() {
            let mut g = paths.lock().unwrap();
            let new_count = g.len() + local_dirs.len();
            g.extend(local_dirs);
            drop(g);
            counter.store(new_count as u64, Ordering::Relaxed);
            // Throttled progress emission
            let mut le = last_emit.lock().unwrap();
            if le.elapsed().as_millis() > 200 {
                *le = std::time::Instant::now();
                let _ = tx.send(IndexMsg::Progress {
                    count: counter.load(Ordering::Relaxed),
                    current: dir.to_string_lossy().to_string(),
                });
            }
        }
        if !subdirs.is_empty() {
            walk_parallel(subdirs, paths, counter, cancel, tx, last_emit);
        }
    });
}
