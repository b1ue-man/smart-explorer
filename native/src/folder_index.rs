// In-memory index of every folder under the chosen scan roots, used to power
// fuzzy folder search ("type 'dwnlds' to jump to Downloads").
//
// Why an index: a live filesystem walk would be far too slow to do on every
// keystroke. Pre-computing paths once gives us O(N) scoring against an
// in-memory array; for ~500k folders this is ~30-80 ms in release builds.
//
// Storage: plain UTF-8 paths, one per line, in %APPDATA%\smart_explorer\
// folder_index.txt. Loading is just split-on-newline.

use crossbeam_channel::Sender;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
#[cfg(windows)]
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

/// Folder names that should never end up in the index. Covers:
///   - Windows system-reserved directories
///   - User-profile noise (AppData, Downloads — mostly cached/installer junk)
///   - Program/system roots (Program Files variants, ProgramData, Windows*)
///   - Dev caches (node_modules)
/// These get skipped whether they appear as a leaf folder or as any segment
/// in a longer path.
const SKIP_NAMES: &[&str] = &[
    // Pure system / recycle
    "$Recycle.Bin",
    "$RECYCLE.BIN",
    "System Volume Information",
    "$WinREAgent",
    "$SysReset",
    "Config.Msi",
    "MSOCache",
    "Recovery",
    "DumpStack.log.tmp",
    // User-profile heavyweight roots
    "AppData",
    "Downloads",
    // Program installs
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    // Windows itself
    "Windows",
    "Windows.old",
    "WinSxS",
    "PerfLogs",
    // Common dev cache (always noise, never navigation target)
    "node_modules",
];

/// Folder names that look auto-generated (hashes, UUIDs, build caches) and
/// shouldn't pollute the navigation index. Heuristics:
///   1. Pure-hex of length >= 8  (git hashes, npm/cargo cache keys, etc.)
///   2. UUID — 8-4-4-4-12 hex with dashes
///   3. Long base64-ish (>= 16 chars, only [A-Za-z0-9_-.=]) with very few
///      vowels (<12%), looks like an encoded ID rather than a word
pub fn is_generic_id(name: &str) -> bool {
    let n = name.len();
    if n < 8 {
        return false;
    }
    let bytes = name.as_bytes();

    // Rule 1: pure hex string of length >= 8
    let mut has_letter = false;
    let mut has_digit = false;
    let mut all_hex = true;
    for &b in bytes {
        match b {
            b'0'..=b'9' => has_digit = true,
            b'a'..=b'f' | b'A'..=b'F' => has_letter = true,
            _ => { all_hex = false; break; }
        }
    }
    if all_hex && has_letter && has_digit {
        return true;
    }
    // Pure-numeric of length >= 12 (probably an ID/timestamp folder)
    if n >= 12 && bytes.iter().all(|b| b.is_ascii_digit()) {
        return true;
    }

    // Rule 2: UUID 8-4-4-4-12 with hex digits
    if n == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
    {
        let only_hex_dash = bytes.iter().all(|&b| {
            matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'-')
        });
        if only_hex_dash {
            return true;
        }
    }

    // Rule 3: long base64-ish with very few vowels
    if n >= 16 {
        let is_alnum_or_meta = bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'='));
        if is_alnum_or_meta {
            let n_vowels = bytes
                .iter()
                .filter(|&&b| matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'))
                .count();
            if (n_vowels * 100) / n < 12 {
                return true;
            }
        }
    }

    false
}

/// True if this folder name should be excluded from the index.
pub fn should_skip(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    if SKIP_NAMES.iter().any(|s| s.eq_ignore_ascii_case(name)) {
        return true;
    }
    if is_generic_id(name) {
        return true;
    }
    false
}

#[cfg(windows)]
pub fn should_skip_meta(name: &str, attrs: u32) -> bool {
    if attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
        return true;
    }
    should_skip(name)
}

#[cfg(not(windows))]
pub fn should_skip_meta(name: &str, _attrs: u32) -> bool {
    should_skip(name)
}

/// True if any segment of `path` (separated by `/`) would be filtered out.
/// Used to clean legacy indices on load.
pub fn path_has_skipped_segment(path: &str) -> bool {
    path.split('/').any(|seg| !seg.is_empty() && should_skip(seg))
}

pub struct FolderIndex {
    /// Absolute folder paths with forward slashes, case-preserving.
    /// HashSet so live updates from the filesystem watcher (insert / remove)
    /// are O(1) even at 500k+ entries.
    paths: HashSet<String>,
    /// Modified-time of the saved index file, if loaded from disk.
    pub built_at: Option<SystemTime>,
}

pub enum IndexMsg {
    Progress { count: u64, current: String },
    Done(FolderIndex),
    Error(String),
}

impl FolderIndex {
    pub fn new() -> Self {
        Self { paths: HashSet::new(), built_at: None }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Insert a path. Returns true if new.
    pub fn insert(&mut self, path: String) -> bool {
        self.paths.insert(path)
    }

    /// Remove a path. Returns true if removed.
    pub fn remove(&mut self, path: &str) -> bool {
        self.paths.remove(path)
    }

    /// True if the index contains exactly this path (no prefix matching).
    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    /// Iterate all indexed paths. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.paths.iter()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut buf = String::with_capacity(self.paths.len() * 50);
        for p in &self.paths {
            buf.push_str(p);
            buf.push('\n');
        }
        std::fs::write(path, buf)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        // While loading, drop any entries that contain a skip-matching segment
        // anywhere in their path. This cleans up legacy indices built before
        // the filter existed — no rebuild needed.
        let paths: HashSet<String> = content
            .lines()
            .filter(|l| !l.is_empty())
            .filter(|l| !path_has_skipped_segment(l))
            .map(|l| l.to_string())
            .collect();
        let built_at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(Self { paths, built_at })
    }

    /// Build an index by walking the given roots in parallel, collecting only
    /// directories. Sends progress over `tx` and posts the final result.
    pub fn build_async(
        roots: Vec<PathBuf>,
        tx: Sender<IndexMsg>,
        cancel: Arc<AtomicBool>,
    ) {
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
                    walk_folders(
                        root.clone(),
                        &paths,
                        &counter,
                        &cancel,
                        &tx,
                        &last_emit,
                    );
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

    /// Score every path against `query`. Returns the top `max` matches sorted
    /// by (fuzzy_score DESC, mtime DESC) — fuzzy score is the primary key, with
    /// last-modified time as tiebreaker (also reorders within a score band so
    /// recently-touched folders surface first).
    ///
    /// To keep stat() calls cheap, we only stat the top `max * 3` fuzzy
    /// candidates and discard the rest.
    pub fn search(&self, query: &str, max: usize) -> Vec<(String, i32)> {
        stat_and_rank(self.search_scored(query, max * 3), max)
    }

    /// CPU-only part of the search: parallel fuzzy scoring, sorted by score
    /// descending, truncated to `n`. No filesystem access — safe to run on
    /// the UI thread even with a cold disk.
    pub fn search_scored(&self, query: &str, n: usize) -> Vec<(String, i32)> {
        if query.is_empty() || self.paths.is_empty() {
            return Vec::new();
        }
        let q_lower: Vec<u8> = query.bytes().map(|b| b.to_ascii_lowercase()).collect();

        let mut scored: Vec<(String, i32)> = self
            .paths
            .par_iter()
            .filter_map(|p| fuzzy_score(&q_lower, p.as_bytes()).map(|s| (p.clone(), s)))
            .collect();
        scored.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        scored.truncate(n);
        scored
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

/// Subsequence fuzzy scoring. Returns Some(score) if all query chars appear in
/// `target` in order (case-insensitive), or None otherwise.
///
/// Heuristics that make scores feel right for path search:
///   - Consecutive matched chars: +3 each (longer runs = better)
///   - Match right after a separator (`/`, `\`, `_`, `-`, `.`, ` `): +8
///   - Match in basename (after last `/`): +30 total bonus
///   - Match in initial char of target: +12
///   - Gap penalty: -1 per unmatched char
///   - Late matches penalty: lower index = better (subtle)
fn fuzzy_score(query: &[u8], target: &[u8]) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if target.is_empty() {
        return None;
    }

    // Find basename start (after last '/')
    let basename_start = target
        .iter()
        .rposition(|&b| b == b'/' || b == b'\\')
        .map(|i| i + 1)
        .unwrap_or(0);

    let mut score: i32 = 0;
    let mut consecutive: i32 = 0;
    let mut qi = 0;
    let mut last_match_idx: Option<usize> = None;
    let mut matched_in_basename = false;

    for (ti, &tc) in target.iter().enumerate() {
        if qi >= query.len() {
            break;
        }
        let tc_l = tc.to_ascii_lowercase();
        let qc_l = query[qi];

        if tc_l == qc_l {
            score += 4;
            if let Some(last) = last_match_idx {
                if last + 1 == ti {
                    consecutive += 1;
                    score += 3 * consecutive;
                } else {
                    consecutive = 0;
                    // Gap penalty proportional to distance, capped
                    let gap = (ti - last - 1).min(20) as i32;
                    score -= gap;
                }
            } else if ti == 0 {
                score += 12;
            }
            // Word-start bonus
            if ti > 0
                && matches!(
                    target[ti - 1],
                    b'/' | b'\\' | b'_' | b'-' | b'.' | b' '
                )
            {
                score += 8;
            }
            if ti >= basename_start {
                if !matched_in_basename {
                    matched_in_basename = true;
                    score += 30;
                }
            }
            last_match_idx = Some(ti);
            qi += 1;
        }
    }

    if qi == query.len() {
        // Bonus for shorter target (more relevant), capped
        let len_penalty = (target.len() as i32 / 8).min(20);
        Some(score - len_penalty)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(s: &str) -> i32 {
        fuzzy_score(s.to_lowercase().as_bytes(), s.as_bytes()).unwrap_or(0)
    }

    #[test]
    fn basic() {
        // Identical match scores higher than substring
        assert!(s("Downloads") > 0);
        // "dnlds" matches "Downloads" but lower than "downloads"
        let exact = fuzzy_score(
            b"downloads",
            b"C:/Users/Silas/Downloads".as_ref(),
        )
        .unwrap();
        let fuzzy = fuzzy_score(b"dnlds", b"C:/Users/Silas/Downloads".as_ref()).unwrap();
        assert!(exact > fuzzy);
    }

    #[test]
    fn no_match() {
        assert!(fuzzy_score(b"xyz", b"abc").is_none());
    }
}
