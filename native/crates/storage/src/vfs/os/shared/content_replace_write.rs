//! One file of a replacement, through the backend of its place: checked
//! against the state the search saw (size, time), skipped with the reason
//! when write-protected, binary, too large for memory or not readable, read
//! whole, replaced in memory, its original kept in the journal, then the new
//! content written to a stage next to it and published with
//! `promote_staged` only if the file is still in the state it was read in
//! (checked right before). The stage carries the file's permission bits.
use std::io::{self, Read, Write};

use crate::agent_proto::{HitRef, Pick, ReplaceEvent, ReplaceSkip};

use super::content_replace::Run;
use super::{Backend, Scheme, StageDurability, StageFinish, VfsMeta};

/// Purpose in the names of the stages (`<file>.se-text-replace-<16 hex>`).
const STAGE_PURPOSE: &str = "text-replace";
/// Attempts at a stage name on a backend whose stages are created exclusively.
const STAGE_ATTEMPTS: usize = 3;

/// A file to replace in.
pub(super) struct Candidate {
    /// Relative to the root of the run.
    pub(super) rel: String,
    /// Size and time as the search or the selected hits saw them.
    pub(super) listed: Option<(u64, i64)>,
    /// Only these hits; `None` = every match.
    pub(super) hits: Option<Vec<HitRef>>,
}

pub(super) fn process(run: &Run<'_>, candidate: Candidate) {
    let rel = candidate.rel.as_str();
    if !valid_rel(rel, run.root_is_file) {
        run.skip(rel, ReplaceSkip::NotRegular);
        return;
    }
    let path = run.path(rel);
    let meta = match run.backend.stat(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let reason = match candidate.listed {
                Some(_) => ReplaceSkip::Changed,
                None => ReplaceSkip::NotRegular,
            };
            run.skip(rel, reason);
            return;
        }
        Err(error) => {
            run.skip(rel, ReplaceSkip::Unreadable(error.to_string()));
            return;
        }
    };
    if meta.is_dir || meta.is_symlink || meta.special {
        run.skip(rel, ReplaceSkip::NotRegular);
        return;
    }
    if let Some((size, mtime_ms)) = candidate.listed {
        let time_differs = mtime_ms != 0 && meta.mtime_ms != 0 && meta.mtime_ms != mtime_ms;
        if meta.size != size || time_differs {
            run.skip(rel, ReplaceSkip::Changed);
            return;
        }
    }
    if read_only(run.backend, &path) {
        run.skip(rel, ReplaceSkip::ReadOnly);
        return;
    }
    let content = match read_whole(run.backend, &path, &meta) {
        Ok(content) => content,
        Err(reason) => {
            run.skip(rel, reason);
            return;
        }
    };
    run.read_one();
    let pick = match &candidate.hits {
        Some(hits) => Pick::Hits(hits),
        None => Pick::All,
    };
    let dry_run = run.request.dry_run;
    let replaced = match run.program.apply(&content, pick, dry_run) {
        Ok(replaced) => replaced,
        Err(reason) => {
            run.skip(rel, reason);
            return;
        }
    };
    if replaced.replacements == 0 {
        return;
    }
    let sizes = (content.len() as u64, replaced.content.len() as u64);
    if dry_run {
        for preview in replaced.previews {
            run.send(ReplaceEvent::Preview {
                rel: rel.to_string(),
                line: preview.line,
                before: preview.before,
                after: preview.after,
                truncated: preview.truncated,
            });
        }
        run.changed(rel, replaced.replacements, sizes);
        return;
    }
    if run.stopped() {
        return;
    }
    let writer = match run.writer() {
        Ok(writer) => writer,
        Err(error) => {
            run.skip(rel, ReplaceSkip::BackupFailed(error));
            return;
        }
    };
    let entry = match writer.backup(
        (rel, &path),
        &content,
        &replaced.content,
        replaced.replacements,
    ) {
        Ok(entry) => entry,
        Err(error) => {
            run.skip(rel, ReplaceSkip::BackupFailed(error.to_string()));
            return;
        }
    };
    match publish(run.backend, &path, &replaced.content, &meta) {
        Ok(()) => {
            writer.applied(entry);
            run.changed(rel, replaced.replacements, sizes);
        }
        Err(PublishError::Changed) => {
            writer.abandon(entry);
            run.skip(rel, ReplaceSkip::Changed);
        }
        Err(PublishError::Failed(error)) => {
            writer.abandon(entry);
            run.skip(rel, ReplaceSkip::WriteFailed(error));
        }
    }
}

/// A relative path that stays below the root: no empty, `.` or `..` part
/// (also between backslashes, which Windows reads as separators), no NUL.
fn valid_rel(rel: &str, root_is_file: bool) -> bool {
    if rel.is_empty() {
        return root_is_file;
    }
    !rel.starts_with('/')
        && !rel.contains('\0')
        && rel
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Write-protected as Notepad++ sees it: no write permission bit where the
/// backend reports Unix modes, else the read-only attribute of a local file.
fn read_only(backend: &dyn Backend, path: &str) -> bool {
    if let Ok(Some(mode)) = super::unix_mode(backend, path) {
        return mode & 0o222 == 0;
    }
    if backend.scheme() != Scheme::Local || !backend.is_local() {
        return false;
    }
    let native = super::local_platform::to_os(path);
    native.is_absolute()
        && std::fs::symlink_metadata(&native).is_ok_and(|meta| meta.permissions().readonly())
}

/// The whole content of the regular file `path` (listed as `meta`).
pub(super) fn read_whole(
    backend: &dyn Backend,
    path: &str,
    meta: &VfsMeta,
) -> Result<Vec<u8>, ReplaceSkip> {
    match backend.read_size(path, meta.size) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Err(ReplaceSkip::Unreadable(
                "der Inhalt wird beim Lesen umgewandelt".to_string(),
            ))
        }
        Err(error) => return Err(ReplaceSkip::Unreadable(error.to_string())),
    }
    let size = usize::try_from(meta.size).map_err(|_| ReplaceSkip::TooLarge)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| ReplaceSkip::TooLarge)?;
    let unreadable = |error: io::Error| ReplaceSkip::Unreadable(error.to_string());
    let reader = super::open_read_regular(backend, path, meta.id.as_deref()).map_err(unreadable)?;
    reader
        .take(meta.size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 != meta.size {
        return Err(ReplaceSkip::Changed);
    }
    Ok(bytes)
}

/// Why new content was not published.
pub(super) enum PublishError {
    /// The file is no longer in the state it was read in.
    Changed,
    Failed(String),
}

/// Publishes `content` at `path` if the file is still as `expected` says.
pub(super) fn publish(
    backend: &dyn Backend,
    path: &str,
    content: &[u8],
    expected: &VfsMeta,
) -> Result<(), PublishError> {
    let failed = |error: io::Error| PublishError::Failed(error.to_string());
    let mode = super::unix_mode(backend, path).ok().flatten();
    let mut attempt = 0;
    let (stage, mut writer) = loop {
        let stage = super::unique_staging_path(backend, path, STAGE_PURPOSE).map_err(failed)?;
        match backend.open_write_copy_stage_sized(&stage, content.len() as u64) {
            Ok(writer) => break (stage, writer),
            // A name another writer took first (never ours to clean up).
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists && attempt + 1 < STAGE_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(error) => return Err(failed(error)),
        }
    };
    let written = (|| {
        writer.write_all(content)?;
        writer.flush()?;
        drop(writer);
        let finish = StageFinish {
            mtime_ms: None,
            mode,
            durability: StageDurability::Now,
        };
        super::finish_stage(backend, &stage, finish).map(drop)
    })();
    if let Err(error) = written {
        discard(backend, &stage);
        return Err(failed(error));
    }
    let unchanged = backend.stat(path).is_ok_and(|now| {
        !now.is_dir
            && !now.is_symlink
            && !now.special
            && now.size == expected.size
            && now.mtime_ms == expected.mtime_ms
            && now.id == expected.id
            && now.content_md5 == expected.content_md5
    });
    if !unchanged {
        discard(backend, &stage);
        return Err(PublishError::Changed);
    }
    if let Err(error) = backend.promote_staged(&stage, path) {
        discard(backend, &stage);
        return Err(failed(error));
    }
    Ok(())
}

/// Removes a stage this run created and never published.
fn discard(backend: &dyn Backend, stage: &str) {
    if let Err(error) = backend.discard_copy_stage(stage) {
        if error.kind() == io::ErrorKind::Unsupported {
            let _ = backend.remove_file(stage);
        }
    }
}
