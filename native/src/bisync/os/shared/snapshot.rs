use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::paths::{join, rel_of};
use super::snapshot_hash::hash_file;
pub use super::snapshot_hash::HashMode;
pub(super) use super::snapshot_hash::{hash_mode, md5_hex_to_u64, md5_to_u64};
use super::types::{Baseline, Sig, Tree};

const MAX_WALK_NODES: u64 = 1_000_000;
const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 512;

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

/// One side's last-known tree (rel → Sig) reconstructed from the saved baseline,
/// used by `walk_files` to reuse stored hashes for files whose size+mtime are
/// unchanged (so a large local tree isn't re-hashed on every run).
pub(super) fn prev_side(base: &Baseline, side_a: bool) -> Tree {
    base.iter()
        .filter_map(|(rel, (a, b))| (if side_a { *a } else { *b }).map(|s| (rel.clone(), s)))
        .collect()
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
    walk_files_impl(be, root, cancel, filter, hash, prev, false)
}

/// Mirror destinations on ID-addressed providers may contain pre-existing
/// duplicate regular-file names. The caller must preflight and apply an exact
/// dedupe plan before any path-based writes; this walk only selects the same
/// deterministic newest ID for planning.
pub(super) fn walk_files_with_duplicate_files(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
) -> io::Result<Tree> {
    walk_files_impl(be, root, cancel, filter, hash, prev, true)
}

#[allow(clippy::too_many_arguments)]
fn walk_files_impl(
    be: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    prev: Option<&Tree>,
    allow_duplicate_files: bool,
) -> io::Result<Tree> {
    let canceled = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "synchronization tree walk canceled",
        )
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    // Fast path: when the backend can produce the signature SERVER-SIDE (the SSH
    // agent's WalkHashed), get the whole tree — including content MD5 for Full —
    // in one pass without downloading a single file. Falls through to the per-dir
    // walk if it didn't run.
    if be.supports_walk_hashed() {
        if let Some(tree) =
            super::snapshot_agent::walk_hashed_via_agent(be, root, cancel, filter, hash)?
        {
            return Ok(tree);
        }
    }

    let par = be.parallelism().max(1);
    let out: Mutex<Tree> = Mutex::new(Tree::new());
    let mut level = vec![root.to_string()];
    let nodes = AtomicU64::new(1);
    let text_bytes = AtomicU64::new(root.len() as u64);
    let mut depth = 0usize;

    while !level.is_empty() {
        if depth > MAX_WALK_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sync tree exceeds {MAX_WALK_DEPTH} levels"),
            ));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
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
                            let mut files: Vec<(String, Sig, String)> = Vec::new();
                            let mut dirs: Vec<String> = Vec::new();
                            let mut child_names: std::collections::HashMap<
                                String,
                                (bool, std::collections::HashSet<String>),
                            > = std::collections::HashMap::new();
                            for m in entries {
                                if cancel.load(Ordering::Relaxed) {
                                    break; // stop promptly mid-directory (esp. when hashing)
                                }
                                if let Err(error) = crate::vfs::validate_child_name(&m.name) {
                                    let mut slot = first_err
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(error);
                                    }
                                    return;
                                }
                                let id = m.id.clone();
                                let duplicate_invalid = match child_names.get_mut(&m.name) {
                                    None => {
                                        let mut ids = std::collections::HashSet::new();
                                        if let Some(id) = id.as_ref() {
                                            ids.insert(id.clone());
                                        }
                                        child_names.insert(
                                            m.name.clone(),
                                            (m.is_dir || m.is_symlink, ids),
                                        );
                                        false
                                    }
                                    Some((prior_non_regular, ids)) => {
                                        !allow_duplicate_files
                                            || *prior_non_regular
                                            || m.is_dir
                                            || m.is_symlink
                                            || id.as_ref().is_none_or(|id| !ids.insert(id.clone()))
                                    }
                                };
                                if duplicate_invalid {
                                    let mut slot = first_err
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            format!(
                                                "backend returned duplicate child name in {dir}: {:?}",
                                                m.name
                                            ),
                                        ));
                                    }
                                    return;
                                }
                                if m.is_symlink {
                                    let mut slot = first_err
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            format!("link-like sync source is unsupported: {dir}/{}", m.name),
                                        ));
                                    }
                                    return;
                                }
                                if !filter.include_hidden && m.hidden {
                                    continue;
                                }
                                let p = join(dir, &m.name);
                                if nodes.fetch_add(1, Ordering::Relaxed) >= MAX_WALK_NODES
                                    || text_bytes.fetch_add(p.len() as u64, Ordering::Relaxed)
                                        > MAX_WALK_TEXT_BYTES.saturating_sub(p.len() as u64)
                                {
                                    let mut slot = first_err
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                                    if slot.is_none() {
                                        *slot = Some(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "sync tree exceeds its bounded collection budget",
                                        ));
                                    }
                                    return;
                                }
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
                                                match hash_file(be, &p, cancel) {
                                                    Ok(hash) => hash,
                                                    Err(error) => {
                                                        let mut slot = first_err.lock().unwrap_or_else(
                                                            |poisoned| poisoned.into_inner(),
                                                        );
                                                        if slot.is_none() {
                                                            *slot = Some(io::Error::new(
                                                                error.kind(),
                                                                format!("hash {p}: {error}"),
                                                            ));
                                                        }
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                        HashMode::FullFresh => {
                                            let native = m
                                                .content_md5
                                                .as_deref()
                                                .map(md5_hex_to_u64)
                                                .unwrap_or(0);
                                            if native != 0 {
                                                native
                                            } else {
                                                match hash_file(be, &p, cancel) {
                                                    Ok(hash) => hash,
                                                    Err(error) => {
                                                        let mut slot = first_err.lock().unwrap_or_else(
                                                            |poisoned| poisoned.into_inner(),
                                                        );
                                                        if slot.is_none() {
                                                            *slot = Some(io::Error::new(
                                                                error.kind(),
                                                                format!("fresh checksum {p}: {error}"),
                                                            ));
                                                        }
                                                        return;
                                                    }
                                                }
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
                                        id.unwrap_or_default(),
                                    ));
                                }
                            }
                            if !files.is_empty() {
                                let mut o =
                                    out.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                                files.sort_by(|left, right| {
                                    left.0
                                        .cmp(&right.0)
                                        .then_with(|| right.1.mtime_ms.cmp(&left.1.mtime_ms))
                                        .then_with(|| left.2.cmp(&right.2))
                                });
                                let mut prior_rel: Option<String> = None;
                                for (rel, sig, _) in files {
                                    if prior_rel.as_deref() != Some(&rel) {
                                        o.insert(rel.clone(), sig);
                                        prior_rel = Some(rel);
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

        // A worker can observe cancellation while it is part-way through a
        // directory. Never turn that partial level into a successful snapshot:
        // callers persist successful walks as the next deletion baseline.
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }

        if let Some(e) = first_err
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return Err(e);
        }
        level = next
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        depth += 1;
    }
    Ok(out
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner()))
}
