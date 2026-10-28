//! A stage filled from bytes held in memory (merged text), published like
//! any copy stage.
use super::apply_guard::{capture, drift, CapturedFile, ExpectedFile};
use super::apply_stage::{flush_unconfirmed_stage, Staged};
use super::paths::parent_of;
use super::transfer_stream::{check, Streamed};
use super::types::Sig;
use crate::vfs::{Backend, StageDurability, StageFinish};
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;

pub(super) fn stage_bytes<'a>(
    destination: &'a dyn Backend,
    path: &str,
    current: &CapturedFile,
    bytes: &[u8],
    mtime_ms: i64,
    cancel: &AtomicBool,
) -> io::Result<Staged<'a>> {
    check(cancel)?;
    if let Some(parent) = parent_of(path) {
        super::apply_path_context::ensure_parent(destination, &parent)?;
    }
    let stage = crate::vfs::unique_staging_path(destination, path, "merge")?;
    let mut staged = Staged {
        backend: destination,
        path: stage,
        bytes: Streamed {
            bytes: bytes.len() as u64,
            digest: md5::compute(bytes).0,
        },
        source: Sig {
            size: bytes.len() as u64,
            mtime_ms,
            hash: super::snapshot_hash::md5_to_u64(&md5::compute(bytes).0),
        },
        durability: StageDurability::Now,
        published: false,
        replacement: None,
    };
    let mode = if current.metadata.is_some() {
        crate::vfs::unix_mode(destination, path)?
    } else {
        Some(0o600)
    };
    let mut writer = crate::vfs::open_write_copy_stage_timed(
        destination,
        &staged.path,
        bytes.len() as u64,
        mtime_ms,
    )?;
    for block in bytes.chunks(256 * 1024) {
        check(cancel)?;
        writer.write_all(block)?;
    }
    writer.flush()?;
    drop(writer);
    let meta = capture(
        destination,
        &staged.path,
        ExpectedFile::Unknown,
        "merged stage",
    )?;
    if meta.regular("merged stage")?.size != bytes.len() as u64 {
        return Err(drift("merged stage size differs"));
    }
    let finished = crate::vfs::finish_stage(
        destination,
        &staged.path,
        StageFinish {
            mtime_ms: Some(mtime_ms),
            mode: mode.map(|mode| mode & 0o777),
            durability: StageDurability::Now,
        },
    )?;
    if destination.is_local() && !finished.durable {
        flush_unconfirmed_stage(destination, &staged.path)?;
    }
    staged.bytes.digest = md5::compute(bytes).0;
    Ok(staged)
}
