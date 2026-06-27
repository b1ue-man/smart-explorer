use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::paths::{join, rel_of};
use super::types::{Baseline, CompareMode, Sig, Tree};

/// What to skip while walking: hidden files, ignore globs (matched on the
/// relative path), and size/age bounds (Group G). A bound of 0 means "no limit".
pub struct WalkFilter<'a> {
    pub include_hidden: bool,
    pub ignore: &'a globset::GlobSet,
    /// Only include files with `min_size <= size <= max_size` (bytes; 0 = off).
    pub min_size: u64,
    pub max_size: u64,
    /// Only include files modified within `[after_mtime_ms, before_mtime_ms]`
    /// (unix ms; 0 = off on that side).
    pub after_mtime_ms: i64,
    pub before_mtime_ms: i64,
}

impl<'a> WalkFilter<'a> {
    /// A filter with no size/age bounds (the common case).
    pub fn basic(include_hidden: bool, ignore: &'a globset::GlobSet) -> Self {
        WalkFilter {
            include_hidden,
            ignore,
            min_size: 0,
            max_size: 0,
            after_mtime_ms: 0,
            before_mtime_ms: 0,
        }
    }

    /// Does a file of this size/mtime pass the size & age bounds?
    pub(super) fn size_age_ok(&self, size: u64, mtime_ms: i64) -> bool {
        if self.min_size > 0 && size < self.min_size {
            return false;
        }
        if self.max_size > 0 && size > self.max_size {
            return false;
        }
        if self.after_mtime_ms > 0 && mtime_ms < self.after_mtime_ms {
            return false;
        }
        if self.before_mtime_ms > 0 && mtime_ms > self.before_mtime_ms {
            return false;
        }
        true
    }
}

/// An empty filter (include everything) — handy for tests / "no settings".
pub fn empty_globset() -> globset::GlobSet {
    globset::GlobSetBuilder::new().build().unwrap()
}

/// First 8 bytes of a 16-byte MD5 digest folded into a u64 (the Sig content
/// key). 0 is reserved for "no hash" (an unreadable file or an un-hashed side),
/// so a real digest of all-zero high bytes is bumped to 1.
pub(super) fn md5_to_u64(d: &[u8; 16]) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&d[..8]);
    let h = u64::from_be_bytes(v);
    if h == 0 {
        1
    } else {
        h
    } // reserve 0 for "no hash"
}

/// Parse a hex MD5 string (e.g. Google Drive `md5Checksum`) into the Sig key.
pub(super) fn md5_hex_to_u64(hex: &str) -> u64 {
    let hex = hex.trim();
    if hex.len() < 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return 0;
    }
    let mut d = [0u8; 16];
    for i in 0..16 {
        d[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    md5_to_u64(&d)
}

/// Stream the file through MD5 → Sig key. Used only when the backend does NOT
/// already provide a content hash (so for a local file this is a cheap local
/// read; for a remote without native hashes it's a download — the slow path).
fn hash_file(be: &dyn Backend, path: &str, cancel: &AtomicBool) -> u64 {
    use std::io::Read;
    let mut ctx = md5::Context::new();
    if let Ok(mut r) = be.open_read(path) {
        let mut buf = [0u8; 65536];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return 0; // abort promptly when the user stops
            }
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => ctx.consume(&buf[..n]),
                Err(_) => return 0,
            }
        }
    } else {
        return 0;
    }
    md5_to_u64(&ctx.compute().0)
}

/// How a walk obtains a file's content hash (decided per side by `hash_mode`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashMode {
    /// Don't hash — size/mtime only.
    None,
    /// Use the backend's FREE native hash if present, else leave it 0. Never
    /// reads/downloads file content (so a hash-less remote stays free).
    NativeOnly,
    /// Native hash if present, else read the file to hash it (a cheap local read,
    /// or — only when the user explicitly chose Checksum — a remote download).
    Full,
}

/// Decide how to hash `this` side given the `other` side and the compare mode.
/// The goal: use a content hash whenever it's FREE or CHEAP, so files whose
/// mtime differs but content matches are not re-transferred — without ever
/// downloading a hash-less remote behind the user's back.
pub(super) fn hash_mode(this: &dyn Backend, other: &dyn Backend, compare: CompareMode) -> HashMode {
    match compare {
        // Pure size compare is already transfer-optimal (identical files share a
        // size) and the user asked to ignore everything else → no hashing.
        CompareMode::SizeOnly => HashMode::None,
        // Explicit checksum: this side MUST yield a content hash even if that
        // means reading/downloading it (native hash is still used first).
        CompareMode::Checksum => HashMode::Full,
        // Default size+mtime: mtime is unreliable across systems (a cloud upload
        // gets a fresh modifiedTime). Opportunistically use a content hash when
        // it's free (native) or cheap (a local read to match the OTHER side's
        // free native hash). Never download a hash-less remote here — that's the
        // explicit Checksum mode's job; fall back to mtime+size for it.
        CompareMode::MtimeSize => {
            if this.provides_content_hash() {
                HashMode::NativeOnly
            } else if this.is_local() && other.provides_content_hash() {
                HashMode::Full
            } else {
                HashMode::None
            }
        }
    }
}

/// One side's last-known tree (rel → Sig) reconstructed from the saved baseline,
/// used by `walk_files` to reuse stored hashes for files whose size+mtime are
/// unchanged (so a large local tree isn't re-hashed on every run).
pub(super) fn prev_side(base: &Baseline, side_a: bool) -> Tree {
    base.iter()
        .filter_map(|(rel, (a, b))| (if side_a { *a } else { *b }).map(|s| (rel.clone(), s)))
        .collect()
}

/// Recursively list files (not dirs) of a backend subtree → rel → Sig,
/// honouring the hidden/ignore filter.
///
/// The walk is breadth-first and **fans out each level across the backend's
/// `parallelism()`** — decisive for remotes like Drive where every `list_dir`
/// is a network round-trip and a 27k-file tree spans hundreds of folders.
/// Build the signature `Tree` from the agent's one-pass server-side walk
/// (`Backend::walk_hashed`), applying the same client-side `filter` the per-dir
/// walk would. `Some` = ran server-side; `None` = backend declined → caller
/// falls back. MD5 is requested only for `HashMode::Full` (so `NativeOnly`/`None`
/// keep their "don't hash" semantics — SFTP has no free hash). The agent's MD5
/// hex maps through `md5_hex_to_u64`, the same key the local side derives from
/// `hash_file`, so cross-side compares stay correct.
fn walk_hashed_via_agent(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
) -> Option<Tree> {
    let want_hash = matches!(hash, HashMode::Full);
    let (tx, rx) = crossbeam_channel::unbounded::<crate::vfs::HashHit>();
    let mut tree = Tree::new();
    let ran = std::thread::scope(|scope| {
        let h = scope.spawn(|| be.walk_hashed(root, want_hash, tx, cancel));
        for hit in rx.iter() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if hit.is_dir {
                continue; // the Tree tracks files
            }
            if filter.ignore.is_match(&hit.rel) {
                continue;
            }
            // Unix dotfile = hidden (the agent doesn't carry an attribute).
            let hidden = hit
                .rel
                .rsplit('/')
                .next()
                .is_some_and(|n| n.starts_with('.'));
            if !filter.include_hidden && hidden {
                continue;
            }
            if !filter.size_age_ok(hit.size, hit.mtime_ms) {
                continue;
            }
            let h = hit.md5.as_deref().map(md5_hex_to_u64).unwrap_or(0);
            tree.insert(
                hit.rel,
                Sig {
                    size: hit.size,
                    mtime_ms: hit.mtime_ms,
                    hash: h,
                },
            );
        }
        h.join().unwrap_or(false)
    });
    if ran {
        Some(tree)
    } else {
        None
    }
}

/// Backends that report `parallelism() == 1` (SFTP/FTP) stay effectively
/// serial. Local uses all cores.
///
/// `hash` chooses the content-hash strategy (see `HashMode`). `prev` is the
/// previous run's tree for THIS side (from the saved baseline): when a file's
/// size+mtime are unchanged from `prev` we reuse its stored hash instead of
/// re-reading the file — so re-hashing a large local tree every sync is avoided.
pub fn walk_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    // Fast path: when the backend can produce the signature SERVER-SIDE (the SSH
    // agent's WalkHashed), get the whole tree — including content MD5 for Full —
    // in one pass without downloading a single file. Falls through to the per-dir
    // walk if it didn't run.
    if be.supports_walk_hashed() {
        if let Some(tree) = walk_hashed_via_agent(be, root, cancel, filter, hash) {
            return Ok(tree);
        }
    }

    let par = be.parallelism().max(1);
    let out: Mutex<Tree> = Mutex::new(Tree::new());
    let mut level = vec![root.to_string()];

    while !level.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let next: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let first_err: Mutex<Option<io::Error>> = Mutex::new(None);
        let idx = AtomicUsize::new(0);
        let workers = par.min(level.len()).max(1);

        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    if cancel.load(Ordering::Relaxed)
                        || first_err
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .is_some()
                    {
                        break;
                    }
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= level.len() {
                        break;
                    }
                    let dir = &level[i];
                    match be.list_dir(dir) {
                        Ok(entries) => {
                            let mut files: Vec<(String, Sig)> = Vec::new();
                            let mut dirs: Vec<String> = Vec::new();
                            for m in entries {
                                if cancel.load(Ordering::Relaxed) {
                                    break; // stop promptly mid-directory (esp. when hashing)
                                }
                                if !filter.include_hidden && m.hidden {
                                    continue;
                                }
                                let p = join(dir, &m.name);
                                let rel = rel_of(&p, root);
                                if filter.ignore.is_match(&rel) {
                                    continue;
                                }
                                if m.is_dir {
                                    if !m.is_symlink {
                                        dirs.push(p);
                                    }
                                } else if filter.size_age_ok(m.size, m.mtime_ms) {
                                    // Content hash, cheapest source first:
                                    //  1. the backend's FREE native MD5
                                    //     (Drive md5Checksum / Nextcloud
                                    //     oc:checksums) — no download;
                                    //  2. the previous run's hash, reused when
                                    //     size+mtime are unchanged — no re-read;
                                    //  3. read the file to hash it (Full only —
                                    //     a cheap local read, or an explicit
                                    //     Checksum-mode remote download).
                                    let h = match hash {
                                        HashMode::None => 0,
                                        HashMode::NativeOnly => m
                                            .content_md5
                                            .as_deref()
                                            .map(md5_hex_to_u64)
                                            .unwrap_or(0),
                                        HashMode::Full => {
                                            if let Some(hex) = m.content_md5.as_deref() {
                                                md5_hex_to_u64(hex)
                                            } else if let Some(ph) = prev
                                                .and_then(|t| t.get(&rel))
                                                .filter(|s| {
                                                    s.size == m.size
                                                        && s.mtime_ms == m.mtime_ms
                                                        && s.hash != 0
                                                })
                                                .map(|s| s.hash)
                                            {
                                                ph
                                            } else {
                                                hash_file(be, &p, cancel)
                                            }
                                        }
                                    };
                                    files.push((
                                        rel,
                                        Sig {
                                            size: m.size,
                                            mtime_ms: m.mtime_ms,
                                            hash: h,
                                        },
                                    ));
                                }
                            }
                            if !files.is_empty() {
                                let mut o =
                                    out.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                                // A backend may report two files with the same
                                // name in one folder (e.g. Google Drive keys by
                                // id, not name). Keep the newest deterministically
                                // so the plan is stable rather than order-dependent.
                                for (rel, sig) in files {
                                    match o.get(&rel) {
                                        Some(prev) if prev.mtime_ms >= sig.mtime_ms => {}
                                        _ => {
                                            o.insert(rel, sig);
                                        }
                                    }
                                }
                            }
                            if !dirs.is_empty() {
                                next.lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .extend(dirs);
                            }
                        }
                        Err(e) => {
                            let mut slot = first_err
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if slot.is_none() {
                                *slot = Some(e);
                            }
                            break;
                        }
                    }
                });
            }
        });

        if let Some(e) = first_err
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return Err(e);
        }
        level = next
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    Ok(out
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner()))
}
