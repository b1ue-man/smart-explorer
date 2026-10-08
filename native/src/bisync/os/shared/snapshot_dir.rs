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

use super::omissions::OmissionKind;
use super::omissions::SyncOmissions;
use super::snapshot::WalkFilter;
use super::snapshot_hash::{hash_file, md5_hex_to_u64, md5_to_u64, HashMode};
use super::snapshot_types::DirSet;
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
    pub(super) duplicates: Option<&'a Mutex<super::snapshot_duplicates::DuplicateGroups>>,
    pub(super) filtered: Option<&'a Mutex<Tree>>,
    pub(super) dirs: Option<&'a Mutex<DirSet>>,
    pub(super) opts: super::BisyncOptions,
    pub(super) limits: super::SyncLimits,
    pub(super) nodes: AtomicU64,
    pub(super) text_bytes: AtomicU64,
    /// A remote side reads file contents (checksums) under a permit of its
    /// flow, with this job id; a local side reads freely, as before.
    pub(super) reads: Option<(Arc<Flow>, u64)>,
    /// When the walk last got a listing or read through (overload patience).
    pub(super) progress: Progress,
    /// The job's live log of the run that started this walk.
    pub(super) log: Option<std::sync::Arc<super::run_log::RunLog>>,
}

/// What one folder contributes to the snapshot.
pub(super) struct Listed {
    /// (rel, signature, id) of every file that passed the filters.
    pub(super) files: Vec<(String, Sig, String)>,
    /// Subfolders to walk next.
    pub(super) dirs: Vec<(String, String)>,
}

/// Checks the entries of folder `dir`. Any error fails the whole walk, so a
/// partial tree can never become a baseline; on cancellation the entries
/// seen so far come back and the walk reports the cancellation.
pub(super) fn scan_listing(
    ctx: &WalkContext<'_>,
    dir: &str,
    dir_rel: &str,
    entries: Vec<VfsMeta>,
) -> io::Result<Listed> {
    let entries = super::snapshot_duplicates::observe(ctx, dir, dir_rel, entries)?;
    let mut listed = Listed {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    let mut child_names: HashMap<String, (bool, HashSet<String>)> = HashMap::new();
    for m in entries {
        if ctx.cancel.load(Ordering::Relaxed) {
            break; // stop promptly mid-directory (esp. when hashing)
        }
        if let Err(error) = super::sync_relative_path::validate_component(&m.name) {
            if let Some(omissions) = ctx.omissions {
                if dir_rel.is_empty() {
                    return Err(error);
                }
                omissions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_kind(dir_rel, OmissionKind::NotRepresentable, true);
                continue;
            }
            return Err(error);
        }
        let id = m.id.clone();
        let duplicate_invalid = match child_names.get_mut(&m.name) {
            None => {
                let mut ids = HashSet::new();
                if let Some(id) = id.as_ref() {
                    ids.insert(id.clone());
                }
                child_names.insert(m.name.clone(), (m.is_dir || m.is_symlink || m.special, ids));
                false
            }
            Some((prior_non_regular, ids)) => {
                !ctx.allow_duplicate_files
                    || *prior_non_regular
                    || m.is_dir
                    || m.is_symlink
                    || m.special
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
        let rel = literal_child(dir_rel, &m.name);
        let p = match crate::vfs::sync_child_path(ctx.be, dir, &m.name) {
            Ok(path) => path,
            Err(error) => {
                if let Some(kind) = super::apply_boundary::omitted(&error) {
                    record_omission(ctx, &rel, kind, true);
                    continue;
                }
                return Err(error);
            }
        };
        if ctx.nodes.fetch_add(1, Ordering::Relaxed) >= ctx.limits.walk_entries
            || ctx
                .text_bytes
                .fetch_add(rel.len() as u64, Ordering::Relaxed)
                > ctx.limits.walk_text_bytes.saturating_sub(rel.len() as u64)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sync tree exceeds its bounded collection budget",
            ));
        }
        let excluded = (!ctx.filter.include_hidden && m.hidden)
            || ctx.filter.ignored(&rel, m.is_dir || m.is_symlink);
        match super::snapshot_policy::protected(ctx.be, ctx.root, &p, &m, ctx.opts.cross_mounts) {
            Ok(Some(kind)) => {
                record_omission(ctx, &rel, kind, !excluded && kind.reported_by_default());
                continue;
            }
            Err(error) => {
                if let Some(reason) = crate::vfs::omission_reason(&error) {
                    record_omission(ctx, &rel, reason.into(), !excluded);
                    continue;
                }
                return Err(error);
            }
            Ok(None) => {}
        }
        if excluded || (!m.is_dir && !ctx.filter.size_age_ok(m.size, m.mtime_ms)) {
            if m.is_dir {
                record_omission(ctx, &rel, OmissionKind::Filtered, false);
            } else if let Some(filtered) = ctx.filtered {
                filtered.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    rel,
                    Sig {
                        size: m.size,
                        mtime_ms: m.mtime_ms,
                        hash: 0,
                    },
                );
            }
            continue;
        }
        if m.is_dir {
            if let Some(dirs) = ctx.dirs {
                dirs.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(rel.clone());
            }
            listed.dirs.push((p, rel));
        } else {
            let hash = match signature_hash(ctx, &m, &rel, &p) {
                Ok(hash) => hash,
                Err(error) => {
                    if error.kind() == io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                    let kind = crate::vfs::omission_reason(&error)
                        .map(OmissionKind::from)
                        .unwrap_or(OmissionKind::Unreadable);
                    record_omission(ctx, &rel, kind, true);
                    continue;
                }
            };
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
    let native = m
        .content_md5
        .as_deref()
        .map(md5_hex_to_u64)
        .filter(|hash| *hash != 0);
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
                None => content_hash(ctx, p, m)
                    .map_err(|error| io::Error::new(error.kind(), format!("hash {p}: {error}"))),
            }
        }
        HashMode::FullFresh => match native.unwrap_or(0) {
            0 => content_hash(ctx, p, m).map_err(|error| {
                io::Error::new(error.kind(), format!("fresh checksum {p}: {error}"))
            }),
            hash => Ok(hash),
        },
    }
}

/// Reads one file to hash it: locally as before, on a remote side under a
/// permit of its flow, reporting the streamed bytes to it and reading again
/// after overload.
fn content_hash(ctx: &WalkContext<'_>, path: &str, listed: &VfsMeta) -> io::Result<u64> {
    let expected = Sig {
        size: listed.size,
        mtime_ms: listed.mtime_ms,
        hash: 0,
    };
    let observed = super::apply_guard::capture(
        ctx.be,
        path,
        super::apply_guard::ExpectedFile::Present(expected),
        "listed checksum file",
    )?;
    if observed.regular("listed checksum file")?.id != listed.id {
        return Err(super::apply_guard::drift(
            "listed checksum file identity changed",
        ));
    }
    let result = if let Some((flow, job)) = &ctx.reads {
        under_permits(
            ctx.cancel,
            &ctx.progress,
            || flow.acquire_for(*job, ctx.cancel),
            |permit| hash_streamed(ctx.be, path, ctx.cancel, &|bytes| permit.progress(bytes)),
        )
        .unwrap_or_else(|| Err(canceled()))?
    } else {
        hash_file(ctx.be, path, ctx.cancel)?
    };
    super::apply_guard::revalidate(ctx.be, path, &observed, "listed checksum file")?;
    Ok(result)
}

fn hash_streamed(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> io::Result<u64> {
    let captured = super::apply_guard::capture(
        backend,
        path,
        super::apply_guard::ExpectedFile::Unknown,
        "checksum file",
    )?;
    let meta = captured.regular("checksum file")?;
    let mut reader = crate::vfs::open_read_regular(backend, path, meta.id.as_deref())?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; HASH_BUFFER];
    let mut length = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length = length.saturating_add(read as u64);
        context.consume(&buffer[..read]);
        progress(read as u64);
    }
    if length != meta.size {
        return Err(super::apply_guard::drift("checksum stream length changed"));
    }
    super::apply_guard::revalidate(backend, path, &captured, "checksum file")?;
    Ok(md5_to_u64(&context.compute().0))
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "checksum walk canceled")
}

pub(super) fn record_omission(
    ctx: &WalkContext<'_>,
    rel: &str,
    kind: OmissionKind,
    reported: bool,
) {
    if let Some(omissions) = ctx.omissions {
        omissions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record_kind(rel, kind, reported);
    }
}

pub(super) fn literal_child(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}
