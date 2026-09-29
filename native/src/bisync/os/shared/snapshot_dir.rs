//! One listed folder of a snapshot walk: every entry is checked, filtered and,
//! where the hash mode asks for it, hashed, exactly as the walk always did
//! (budget, duplicate names, protected omissions, hash reuse). The walk in
//! `snapshot.rs` decides which folders are listed when.
use crate::transfer::Flow;
use crate::vfs::{Backend, VfsMeta};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::omissions::SyncOmissions;
use super::paths::{join, rel_of};
use super::snapshot::{WalkFilter, MAX_WALK_NODES, MAX_WALK_TEXT_BYTES};
use super::snapshot_hash::{hash_file, md5_hex_to_u64, md5_to_u64, HashMode};
use super::sync_overload::{under_permits, Progress};
use super::types::{Sig, Tree};

/// Checksum reads stream in blocks of this size (as `hash_file` does).
const HASH_BUFFER: usize = 65_536;

/// Inputs and counters shared by every folder of one walk.
pub(super) struct WalkContext<'a> {
    pub(super) be: &'a dyn Backend,
    pub(super) root: &'a str,
    pub(super) cancel: &'a AtomicBool,
    pub(super) filter: &'a WalkFilter<'a>,
    pub(super) hash: HashMode,
    pub(super) prev: Option<&'a Tree>,
    pub(super) allow_duplicate_files: bool,
    pub(super) omissions: Option<&'a Mutex<SyncOmissions>>,
    pub(super) nodes: AtomicU64,
    pub(super) text_bytes: AtomicU64,
    /// A remote side reads file contents (checksums) under a permit of its
    /// flow, with this job id; a local side reads freely, as before.
    pub(super) reads: Option<(Arc<Flow>, u64)>,
    /// When the walk last got a listing or read through (overload patience).
    pub(super) progress: Progress,
}

/// What one folder contributes to the snapshot.
pub(super) struct Listed {
    /// (rel, signature, id) of every file that passed the filters.
    pub(super) files: Vec<(String, Sig, String)>,
    /// Subfolders to walk next.
    pub(super) dirs: Vec<String>,
}

/// Checks the entries of folder `dir`. Any error fails the whole walk, so a
/// partial tree can never become a baseline; on cancellation the entries
/// seen so far come back and the walk reports the cancellation.
pub(super) fn scan_listing(
    ctx: &WalkContext<'_>,
    dir: &str,
    entries: Vec<VfsMeta>,
) -> io::Result<Listed> {
    let mut listed = Listed {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    let mut child_names: HashMap<String, (bool, HashSet<String>)> = HashMap::new();
    for m in entries {
        if ctx.cancel.load(Ordering::Relaxed) {
            break; // stop promptly mid-directory (esp. when hashing)
        }
        crate::vfs::validate_child_name(&m.name)?;
        let id = m.id.clone();
        let duplicate_invalid = match child_names.get_mut(&m.name) {
            None => {
                let mut ids = HashSet::new();
                if let Some(id) = id.as_ref() {
                    ids.insert(id.clone());
                }
                child_names.insert(m.name.clone(), (m.is_dir || m.is_symlink, ids));
                false
            }
            Some((prior_non_regular, ids)) => {
                !ctx.allow_duplicate_files
                    || *prior_non_regular
                    || m.is_dir
                    || m.is_symlink
                    || id.as_ref().is_none_or(|id| !ids.insert(id.clone()))
            }
        };
        if duplicate_invalid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "backend returned duplicate child name in {dir}: {:?}",
                    m.name
                ),
            ));
        }
        let p = join(dir, &m.name);
        if ctx.nodes.fetch_add(1, Ordering::Relaxed) >= MAX_WALK_NODES
            || ctx.text_bytes.fetch_add(p.len() as u64, Ordering::Relaxed)
                > MAX_WALK_TEXT_BYTES.saturating_sub(p.len() as u64)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sync tree exceeds its bounded collection budget",
            ));
        }
        let rel = rel_of(&p, ctx.root);
        let excluded = (!ctx.filter.include_hidden && m.hidden)
            || ctx.filter.ignored(&rel, m.is_dir || m.is_symlink);
        // The app trash and other apps' private storage (Android) are
        // protected omissions like a link: never synced, their counterparts
        // kept.
        if m.is_symlink
            || crate::apptrash::excluded_name(&m.name)
            || crate::apptrash::hidden_app_folders_in(dir)
        {
            if let Some(omissions) = ctx.omissions {
                omissions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record(&rel, !excluded);
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("link-like sync source requires a protected snapshot: {p}"),
            ));
        }
        if excluded {
            continue;
        }
        if m.is_dir {
            if !m.is_symlink {
                listed.dirs.push(p);
            }
        } else if ctx.filter.size_age_ok(m.size, m.mtime_ms) {
            let hash = signature_hash(ctx, &m, &rel, &p)?;
            listed.files.push((
                rel,
                Sig {
                    size: m.size,
                    mtime_ms: m.mtime_ms,
                    hash,
                },
                id.unwrap_or_default(),
            ));
        }
    }
    Ok(listed)
}

/// Content hash, cheapest source first:
///  1. the backend's FREE native MD5 (Drive md5Checksum / Nextcloud
///     oc:checksums) — no download;
///  2. the previous run's hash, reused when size+mtime are unchanged — no
///     re-read (never in Checksum mode, which demands a fresh hash);
///  3. read the file to hash it (Full only — a cheap local read, or an
///     explicit Checksum-mode remote download).
fn signature_hash(ctx: &WalkContext<'_>, m: &VfsMeta, rel: &str, p: &str) -> io::Result<u64> {
    let native = m.content_md5.as_deref().map(md5_hex_to_u64);
    match ctx.hash {
        HashMode::None => Ok(0),
        HashMode::NativeOnly => Ok(native.unwrap_or(0)),
        HashMode::Full => {
            if let Some(hash) = native {
                return Ok(hash);
            }
            let reused = ctx
                .prev
                .and_then(|tree| tree.get(rel))
                .filter(|sig| sig.size == m.size && sig.mtime_ms == m.mtime_ms && sig.hash != 0)
                .map(|sig| sig.hash);
            match reused {
                Some(hash) => Ok(hash),
                None => content_hash(ctx, p)
                    .map_err(|error| io::Error::new(error.kind(), format!("hash {p}: {error}"))),
            }
        }
        HashMode::FullFresh => match native.unwrap_or(0) {
            0 => content_hash(ctx, p).map_err(|error| {
                io::Error::new(error.kind(), format!("fresh checksum {p}: {error}"))
            }),
            hash => Ok(hash),
        },
    }
}

/// Reads one file to hash it: locally as before, on a remote side under a
/// permit of its flow, reporting the streamed bytes to it and reading again
/// after overload.
fn content_hash(ctx: &WalkContext<'_>, path: &str) -> io::Result<u64> {
    let Some((flow, job)) = &ctx.reads else {
        return hash_file(ctx.be, path, ctx.cancel);
    };
    under_permits(
        ctx.cancel,
        &ctx.progress,
        || flow.acquire_for(*job, ctx.cancel),
        |permit| hash_streamed(ctx.be, path, ctx.cancel, &|bytes| permit.progress(bytes)),
    )
    .unwrap_or_else(|| Err(canceled()))
}

fn hash_streamed(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> io::Result<u64> {
    let mut reader = backend.open_read(path)?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; HASH_BUFFER];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
        progress(read as u64);
    }
    Ok(md5_to_u64(&context.compute().0))
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "checksum walk canceled")
}
