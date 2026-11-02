//! One listed folder of a snapshot walk: every entry is checked, filtered and,
//! where the hash mode asks for it, hashed, exactly as the walk always did
//! (budget, duplicate names, protected omissions, hash reuse). The files of a
//! remote side whose content must be hashed are asked together first
//! (`vfs::content_md5_many`: SFTP without an agent forms their MD5s with one
//! exec request per group, not one per file); a file without such an answer
//! goes the single-file way. The walk in `snapshot.rs` decides which folders
//! are listed when.
use crate::transfer::Flow;
use crate::vfs::{Backend, VfsMeta};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::omissions::OmissionKind;
use super::omissions::SyncOmissions;
use super::snapshot::WalkFilter;
use super::snapshot_hash::{
    hash_file_cached_hex, md5_hex_to_u64, md5_to_u64, HashMode, LocalReads,
};
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
    /// Told about every folder listed completely (G4 MS14).
    pub(super) listed: Option<&'a dyn super::early_copy::FolderListed>,
    /// The listings come from a walk next to the data
    /// (`snapshot_host_walk.rs`): child paths are joined here, and digests
    /// arrive as the listing's MD5, so nothing is asked of the backend.
    pub(super) host_listed: bool,
    /// The facts of a local walk established once (`snapshot_policy`).
    pub(super) local: Option<super::snapshot_policy::LocalFacts>,
    /// A remote side's folder key files (`snapshot_keyed.rs`); `None` reads
    /// without folder keys.
    pub(super) key_files: Option<super::snapshot_keyed::KeyFiles>,
    /// The pair's key phases and this side's index (`snapshot_pair_keys.rs`).
    pub(super) pair: Option<(&'a super::snapshot_pair_keys::PairKeys, usize)>,
    /// A local side of the pair's key phases: this walk forms its folder
    /// keys as it lists (`snapshot_local_keys.rs`).
    pub(super) local_keys: Option<&'a super::snapshot_local_keys::LocalKeys>,
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
    let mut app_parents = super::snapshot_policy::AppParents::default();
    // (rel, path, listing) of every file that passed the filters, hashed below.
    let mut files: Vec<(String, String, VfsMeta)> = Vec::new();
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
        let excluded = (!ctx.filter.include_hidden && m.hidden)
            || ctx.filter.ignored(&rel, m.is_dir || m.is_symlink);
        // Only an entry the filters keep needs the backend's own spelling of
        // its path (a request per entry on a Share host); the protection
        // checks of an excluded one look only at its parent and its depth.
        let child = if excluded || ctx.host_listed {
            crate::vfs::literal_child_path(dir, &m.name)
        } else {
            crate::vfs::sync_child_path(ctx.be, dir, &m.name)
        };
        let p = match child {
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
        let facts = ctx.local.as_ref();
        match super::snapshot_policy::protected_listed(
            ctx.be,
            ctx.root,
            &p,
            &m,
            ctx.opts.cross_mounts,
            facts,
            &mut app_parents,
        ) {
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
            files.push((rel, p, m));
        }
    }
    let batched = batched_md5(ctx, &files)?;
    for ((rel, p, m), batched) in files.into_iter().zip(batched) {
        if ctx.cancel.load(Ordering::Relaxed) {
            break;
        }
        let hash = match signature_hash(ctx, &m, &rel, &p, batched) {
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
            m.id.unwrap_or_default(),
        ));
    }
    Ok(listed)
}

/// The MD5s of the files of one folder whose content must be hashed, asked
/// together where a remote side has a batched way (`vfs::content_md5_many`).
/// An answer proves a regular file with the listed size and time before and
/// after its MD5 was formed (the checks `content_hash` makes around the single
/// request). `None` for every other file: it goes its own way below, with
/// its own errors and omissions. A local side, a walk next to the data
/// (digests in the listing) and an ID-addressed provider are not asked.
fn batched_md5(
    ctx: &WalkContext<'_>,
    files: &[(String, String, VfsMeta)],
) -> io::Result<Vec<Option<u64>>> {
    let mut out = vec![None; files.len()];
    if ctx.be.is_local() || ctx.host_listed || ctx.be.has_duplicate_file_names() {
        return Ok(out);
    }
    let wanted: Vec<usize> = (0..files.len())
        .filter(|index| cheap_hash(ctx, &files[*index].2, &files[*index].0).is_none())
        .collect();
    if wanted.is_empty() {
        return Ok(out);
    }
    let asked: Vec<(&str, &VfsMeta)> = wanted
        .iter()
        .map(|index| (files[*index].1.as_str(), &files[*index].2))
        .collect();
    match crate::vfs::content_md5_many(ctx.be, &asked, ctx.cancel) {
        Ok(Some(found)) => {
            for (index, hex) in wanted.into_iter().zip(found) {
                out[index] = hex
                    .map(|hex| md5_hex_to_u64(&hex))
                    .filter(|hash| *hash != 0);
            }
        }
        Ok(None) => {}
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        // Every file then asks on its own, which reports what went wrong.
        Err(_) => {}
    }
    Ok(out)
}

/// Content hash, cheapest source first:
///  1. the backend's FREE native MD5 (Drive md5Checksum / Nextcloud
///     oc:checksums) — no download;
///  2. the previous run's hash, reused when size+mtime are unchanged — no
///     re-read (never in Checksum mode, which demands a fresh hash);
///  3. the MD5 the folder's batched request brought (`batched`);
///  4. read the file to hash it (Full only — a cheap local read, or an
///     explicit Checksum-mode remote download).
fn signature_hash(
    ctx: &WalkContext<'_>,
    m: &VfsMeta,
    rel: &str,
    p: &str,
    batched: Option<u64>,
) -> io::Result<u64> {
    if let Some(hash) = cheap_hash(ctx, m, rel).or(batched) {
        return Ok(hash);
    }
    let label = if ctx.hash == HashMode::FullFresh {
        "fresh checksum"
    } else {
        "hash"
    };
    content_hash(ctx, (p, rel), m)
        .map_err(|error| io::Error::new(error.kind(), format!("{label} {p}: {error}")))
}

/// Steps 1 and 2 of `signature_hash` (and the modes that hash nothing):
/// `None` when the content must be hashed.
fn cheap_hash(ctx: &WalkContext<'_>, m: &VfsMeta, rel: &str) -> Option<u64> {
    let native = m
        .content_md5
        .as_deref()
        .map(md5_hex_to_u64)
        .filter(|hash| *hash != 0);
    match ctx.hash {
        HashMode::None => Some(0),
        HashMode::NativeOnly => Some(native.unwrap_or(0)),
        HashMode::Full => native.or_else(|| {
            ctx.prev
                .and_then(|tree| tree.get(rel))
                .filter(|sig| sig.size == m.size && sig.mtime_ms == m.mtime_ms && sig.hash != 0)
                .map(|sig| sig.hash)
        }),
        HashMode::FullFresh => native,
    }
}

/// Reads one file to hash it: locally as before, on a remote side under a
/// permit of its flow, reporting the streamed bytes to it and reading again
/// after overload. A local side's folder keys learn the digest
/// (`ctx.local_keys`) once the file proved unchanged by the read.
fn content_hash(
    ctx: &WalkContext<'_>,
    (path, rel): (&str, &str),
    listed: &VfsMeta,
) -> io::Result<u64> {
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
    // The MD5 formed where the file is stored, where the side offers one
    // (`vfs::content_md5`, E9): no download for it.
    let stored = if ctx.be.is_local() {
        None
    } else {
        crate::vfs::content_md5(ctx.be, path, ctx.cancel)?
            .map(|hex| md5_hex_to_u64(&hex))
            .filter(|hash| *hash != 0)
    };
    let (result, hex) = if let Some(hash) = stored {
        (hash, None)
    } else if let Some((flow, job)) = &ctx.reads {
        let hash = under_permits(
            ctx.cancel,
            &ctx.progress,
            || flow.acquire_for(*job, ctx.cancel),
            |permit| hash_streamed(ctx.be, path, ctx.cancel, &|bytes| permit.progress(bytes)),
        )
        .unwrap_or_else(|| Err(canceled()))?;
        (hash, None)
    } else {
        // The one walk of a local side reads each file once per run.
        let reads = LocalReads {
            fresh: ctx.hash == HashMode::FullFresh,
            run: None,
        };
        let hex = hash_file_cached_hex(ctx.be, path, ctx.cancel, reads)?;
        (md5_hex_to_u64(&hex), Some(hex))
    };
    super::apply_guard::revalidate(ctx.be, path, &observed, "listed checksum file")?;
    if let (Some(keys), Some(hex)) = (ctx.local_keys, hex) {
        keys.digest(rel, hex);
    }
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

/// The omissions of one folder's listing, as every walk registers them.
pub(super) fn record_listing_omissions(
    ctx: &WalkContext<'_>,
    dir_rel: &str,
    omitted: Vec<crate::vfs::VfsOmission>,
) -> io::Result<()> {
    for omission in omitted {
        let rel = match super::sync_relative_path::validate_component(&omission.rel) {
            Ok(()) => literal_child(dir_rel, &omission.rel),
            Err(error) if dir_rel.is_empty() => return Err(error),
            Err(_) => dir_rel.to_string(),
        };
        let filtered = ctx.filter.ignored(&rel, true);
        record_omission(ctx, &rel, omission.reason.into(), !filtered);
    }
    Ok(())
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

#[cfg(test)]
#[path = "abgleich_task_batch_md5_tests.rs"]
mod batch_md5_tests;
