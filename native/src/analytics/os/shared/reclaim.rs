//! Find-and-reclaim scan: local cleanup candidates, large/stale files, empty
//! entries, and duplicate groups. The scan is read-only; UI actions decide what
//! to move to the recycle bin.

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ReclaimOptions {
    pub large_min_bytes: u64,
    pub stale_days: u64,
    pub max_items: usize,
}

impl Default for ReclaimOptions {
    fn default() -> Self {
        Self {
            large_min_bytes: 1024 * 1024 * 1024,
            stale_days: 365,
            max_items: 200,
        }
    }
}

#[derive(Clone, Default)]
pub struct ReclaimProgress {
    pub files: Arc<AtomicU64>,
    pub dirs: Arc<AtomicU64>,
    pub bytes: Arc<AtomicU64>,
    pub hashed: Arc<AtomicU64>,
    pub candidates: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimItem {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub is_dir: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub md5: String,
    pub size: u64,
    pub reclaimable: u64,
    pub items: Vec<ReclaimItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReclaimReport {
    pub root: String,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub large_files: Vec<ReclaimItem>,
    pub stale_files: Vec<ReclaimItem>,
    pub empty_files: Vec<ReclaimItem>,
    pub empty_dirs: Vec<ReclaimItem>,
    pub cleanup: Vec<ReclaimItem>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub errors: Vec<String>,
}

impl ReclaimReport {
    pub fn reclaimable_bytes(&self) -> u64 {
        let dup = self
            .duplicate_groups
            .iter()
            .map(|g| g.reclaimable)
            .sum::<u64>();
        let empty = self.empty_files.iter().map(|i| i.size).sum::<u64>();
        let cleanup = self.cleanup.iter().map(|i| i.size).sum::<u64>();
        dup + empty + cleanup
    }

    pub fn prune_paths(&mut self, paths: &[String]) {
        let gone: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        let keep = |i: &ReclaimItem| !gone.contains(i.path.as_str());
        self.large_files.retain(keep);
        self.stale_files.retain(keep);
        self.empty_files.retain(keep);
        self.empty_dirs.retain(keep);
        self.cleanup.retain(keep);
        for g in &mut self.duplicate_groups {
            g.items.retain(keep);
            g.reclaimable = g
                .size
                .saturating_mul(g.items.len().saturating_sub(1) as u64);
        }
        self.duplicate_groups.retain(|g| g.items.len() > 1);
    }
}

#[derive(Clone)]
struct FileCandidate {
    path: PathBuf,
    item: ReclaimItem,
}

#[derive(Default)]
struct Acc {
    files: Vec<FileCandidate>,
    large: Vec<ReclaimItem>,
    stale: Vec<ReclaimItem>,
    empty_files: Vec<ReclaimItem>,
    empty_dirs: Vec<ReclaimItem>,
    cleanup: Vec<ReclaimItem>,
    errors: Vec<String>,
}

pub fn scan_reclaim(
    root: &Path,
    progress: &ReclaimProgress,
    opts: &ReclaimOptions,
) -> ReclaimReport {
    let root_abs = root.to_path_buf();
    let acc = Arc::new(Mutex::new(Acc::default()));
    let cutoff = now_ms().saturating_sub((opts.stale_days as i64) * 86_400_000);
    let bytes = scan_dir(&root_abs, &root_abs, progress, opts, cutoff, &acc);
    let mut acc = match Arc::try_unwrap(acc) {
        Ok(m) => m.into_inner().unwrap_or_default(),
        Err(a) => {
            let mut guard = a.lock().unwrap();
            std::mem::take(&mut *guard)
        }
    };

    acc.large.sort_by_key(|i| std::cmp::Reverse(i.size));
    acc.stale.sort_by_key(|i| std::cmp::Reverse(i.size));
    acc.empty_files.sort_by(|a, b| a.path.cmp(&b.path));
    acc.empty_dirs.sort_by(|a, b| a.path.cmp(&b.path));
    acc.cleanup.sort_by_key(|i| std::cmp::Reverse(i.size));
    truncate(&mut acc.large, opts.max_items);
    truncate(&mut acc.stale, opts.max_items);
    truncate(&mut acc.empty_files, opts.max_items);
    truncate(&mut acc.empty_dirs, opts.max_items);
    truncate(&mut acc.cleanup, opts.max_items);

    let mut duplicate_groups = duplicate_groups(acc.files, progress, opts, &mut acc.errors);
    duplicate_groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    truncate(&mut duplicate_groups, opts.max_items);

    ReclaimReport {
        root: to_fwd(&root_abs),
        files: progress.files.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        bytes,
        large_files: acc.large,
        stale_files: acc.stale,
        empty_files: acc.empty_files,
        empty_dirs: acc.empty_dirs,
        cleanup: acc.cleanup,
        duplicate_groups,
        errors: acc.errors,
    }
}

fn scan_dir(
    dir: &Path,
    root: &Path,
    p: &ReclaimProgress,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    acc: &Arc<Mutex<Acc>>,
) -> u64 {
    if p.cancel.load(Ordering::Relaxed) || crate::agent_proto::is_pseudo_dir(&dir.to_string_lossy())
    {
        return 0;
    }
    p.dirs.fetch_add(1, Ordering::Relaxed);
    let mut subdirs = Vec::new();
    let mut own_size = 0u64;
    let mut child_count = 0usize;

    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            acc.lock()
                .unwrap()
                .errors
                .push(format!("{}: {}", to_fwd(dir), e));
            return 0;
        }
    };

    for ent in rd.flatten() {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let ft = match ent.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                acc.lock()
                    .unwrap()
                    .errors
                    .push(format!("{}: {}", to_fwd(&ent.path()), e));
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        child_count += 1;
        let path = ent.path();
        if ft.is_dir() {
            subdirs.push(path);
        } else if ft.is_file() {
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime_ms = md
                .and_then(|m| m.modified().ok())
                .map(systemtime_ms)
                .unwrap_or(0);
            let name = ent.file_name().to_string_lossy().into_owned();
            let item = ReclaimItem {
                path: to_fwd(&path),
                name,
                size,
                mtime_ms,
                is_dir: false,
                reason: String::new(),
            };
            p.files.fetch_add(1, Ordering::Relaxed);
            p.bytes.fetch_add(size, Ordering::Relaxed);
            own_size = own_size.saturating_add(size);
            record_file(item, path, opts, stale_cutoff_ms, acc);
        }
    }

    let sub_size = if subdirs.len() > 1 {
        subdirs
            .par_iter()
            .map(|d| scan_dir(d, root, p, opts, stale_cutoff_ms, acc))
            .sum()
    } else {
        subdirs
            .iter()
            .map(|d| scan_dir(d, root, p, opts, stale_cutoff_ms, acc))
            .sum()
    };
    let total = own_size.saturating_add(sub_size);
    record_dir(dir, root, total, child_count, acc);
    total
}

fn record_file(
    mut item: ReclaimItem,
    path: PathBuf,
    opts: &ReclaimOptions,
    stale_cutoff_ms: i64,
    acc: &Arc<Mutex<Acc>>,
) {
    let mut acc = acc.lock().unwrap();
    if item.size >= opts.large_min_bytes {
        item.reason = "gross".to_string();
        acc.large.push(item.clone());
    }
    if item.mtime_ms > 0 && item.mtime_ms < stale_cutoff_ms {
        item.reason = "alt".to_string();
        acc.stale.push(item.clone());
    }
    if item.size == 0 {
        item.reason = "leer".to_string();
        acc.empty_files.push(item.clone());
    }
    if let Some(reason) = cleanup_reason(&item.name, false) {
        item.reason = reason.to_string();
        acc.cleanup.push(item.clone());
    }
    acc.files.push(FileCandidate { path, item });
}

fn record_dir(dir: &Path, root: &Path, size: u64, child_count: usize, acc: &Arc<Mutex<Acc>>) {
    if dir == root {
        return;
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut item = ReclaimItem {
        path: to_fwd(dir),
        name,
        size,
        mtime_ms: 0,
        is_dir: true,
        reason: String::new(),
    };
    let mut acc = acc.lock().unwrap();
    if child_count == 0 {
        item.reason = "leerer Ordner".to_string();
        acc.empty_dirs.push(item.clone());
    }
    if let Some(reason) = cleanup_reason(&item.name, true) {
        item.reason = reason.to_string();
        acc.cleanup.push(item);
    }
}

fn duplicate_groups(
    files: Vec<FileCandidate>,
    p: &ReclaimProgress,
    _opts: &ReclaimOptions,
    errors: &mut Vec<String>,
) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<FileCandidate>> = HashMap::new();
    for f in files.into_iter().filter(|f| f.item.size > 0) {
        by_size.entry(f.item.size).or_default().push(f);
    }

    let mut groups = Vec::new();
    for (size, same_size) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut by_hash: HashMap<String, Vec<ReclaimItem>> = HashMap::new();
        for f in same_size {
            if p.cancel.load(Ordering::Relaxed) {
                break;
            }
            match md5_file(&f.path) {
                Ok(h) => {
                    p.hashed.fetch_add(1, Ordering::Relaxed);
                    by_hash.entry(h).or_default().push(f.item);
                }
                Err(e) => errors.push(format!("Hash {}: {}", to_fwd(&f.path), e)),
            }
        }
        for (md5, mut items) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
            items.sort_by_key(|i| std::cmp::Reverse(i.mtime_ms));
            let reclaimable = size.saturating_mul(items.len().saturating_sub(1) as u64);
            p.candidates
                .fetch_add(items.len() as u64, Ordering::Relaxed);
            groups.push(DuplicateGroup {
                md5,
                size,
                reclaimable,
                items,
            });
        }
    }
    groups
}

fn cleanup_reason(name: &str, is_dir: bool) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if is_dir {
        match n.as_str() {
            "node_modules" => Some("node_modules"),
            ".git" => Some(".git"),
            "cache" | "caches" | ".cache" => Some("Cache"),
            "log" | "logs" => Some("Logs"),
            "__pycache__" | ".pytest_cache" | ".mypy_cache" => Some("Python-Cache"),
            ".gradle" => Some("Gradle-Cache"),
            "target" => Some("Rust-Build"),
            _ => None,
        }
    } else if n.ends_with(".log") {
        Some("Logdatei")
    } else {
        None
    }
}

fn md5_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut ctx = md5::Context::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Ok(format!("{:x}", ctx.compute()))
}

fn truncate<T>(v: &mut Vec<T>, max: usize) {
    if v.len() > max {
        v.truncate(max);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

fn systemtime_ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(systemtime_ms_from_duration)
        .unwrap_or(0)
}

fn systemtime_ms_from_duration(d: std::time::Duration) -> i64 {
    d.as_secs() as i64 * 1000 + i64::from(d.subsec_millis())
}

fn to_fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaim_finds_duplicates_empty_and_cleanup() {
        let base = std::env::temp_dir().join(format!("se_reclaim_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(base.join("empty_dir")).unwrap();
        std::fs::write(base.join("a.bin"), b"same").unwrap();
        std::fs::write(base.join("b.bin"), b"same").unwrap();
        std::fs::write(base.join("empty.txt"), b"").unwrap();
        std::fs::write(base.join("node_modules/pkg/cache.js"), b"cached").unwrap();

        let opts = ReclaimOptions {
            large_min_bytes: 1,
            stale_days: 0,
            max_items: 50,
        };
        let p = ReclaimProgress::default();
        let r = scan_reclaim(&base, &p, &opts);

        assert!(r.large_files.iter().any(|i| i.name == "a.bin"));
        assert!(r.stale_files.iter().any(|i| i.name == "b.bin"));
        assert!(r.empty_files.iter().any(|i| i.name == "empty.txt"));
        assert!(r.empty_dirs.iter().any(|i| i.name == "empty_dir"));
        assert!(r.cleanup.iter().any(|i| i.name == "node_modules"));
        assert_eq!(r.duplicate_groups.len(), 1);
        assert_eq!(r.duplicate_groups[0].items.len(), 2);

        let _ = std::fs::remove_dir_all(&base);
    }
}
