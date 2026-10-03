//! Private durable merge intent and individually confirmed publications.
use super::apply_guard::{capture, drift, revalidate, ExpectedFile};
use super::merge_precheck::Compared;
use super::merge_recorded::OriginalContent;
use super::run_types::StateKey;
use super::types::{PairSide, Sig};
use super::versions::{VersionReason, VersionSide};
use crate::vfs::Backend;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Recovery {
    pub pair: String,
    pub lock: String,
    pub rel: String,
    pub kind: String,
    pub original_a: Option<String>,
    pub original_b: Option<String>,
    pub merged: String,
    pub run: String,
    pub started_ms: i64,
    pub a: Option<Sig>,
    pub b: Option<Sig>,
    pub done_a: bool,
    pub done_b: bool,
    pub sibling: Option<String>,
    pub sibling_a: Option<Sig>,
    pub sibling_b: Option<Sig>,
}
fn path(key: &StateKey, rel: &str) -> io::Result<PathBuf> {
    Ok(
        super::replica_state::baseline_file(key)?.with_extension(format!(
            "merge-{}.json",
            super::version_manifest::token(rel)
        )),
    )
}
pub(super) fn load(key: &StateKey, rel: &str) -> io::Result<Option<Recovery>> {
    let path = path(key, rel)?;
    match crate::support_dirs::read_private_text(&path, 256 * 1024) {
        Ok(text) => {
            let recovery: Recovery = serde_json::from_str(&text).map_err(io::Error::other)?;
            if recovery.pair != key.pair_id || recovery.lock != key.lock_id || recovery.rel != rel {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "merge recovery belongs to another state",
                ));
            }
            if let Some(sibling) = &recovery.sibling {
                crate::agent_proto::ValidatedRelativePath::parse(sibling)?;
            }
            Ok(Some(recovery))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
pub(super) fn save(key: &StateKey, recovery: &Recovery) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(recovery).map_err(io::Error::other)?;
    if bytes.len() > 256 * 1024 {
        return Err(io::Error::other("merge recovery exceeds its budget"));
    }
    crate::support_dirs::write_private_atomic(&path(key, &recovery.rel)?, &bytes)
}
pub(super) fn remove(key: &StateKey, rel: &str) -> io::Result<()> {
    let path = path(key, rel)?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("merge recovery path is not Unicode"))?;
    let backend = crate::vfs::LocalBackend::new(text);
    super::apply_stage::require_durable(super::apply_stage::namespace(&backend, text)?)
}

/// Exact owner/replica filenames keep another job's recovery separate.
pub(super) fn relatives(key: &StateKey) -> io::Result<Vec<String>> {
    let base = super::replica_state::baseline_file(key)?;
    let parent = base
        .parent()
        .ok_or_else(|| io::Error::other("merge state has no parent"))?;
    let stem = base
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("merge state is not Unicode"))?;
    let prefix = format!("{stem}.merge-");
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut relatives = std::collections::BTreeSet::new();
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut count = 0u64;
    let mut text = 0u64;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) || !name.ends_with(".json") {
            continue;
        }
        count = count.saturating_add(1);
        text = text.saturating_add(name.len() as u64);
        if count > limits.state_entries || text > limits.state_text_bytes {
            return Err(io::Error::other(
                "pending merge collection exceeds its budget",
            ));
        }
        let bytes = crate::support_dirs::read_private_text(&entry.path(), 256 * 1024)?;
        let recovery: Recovery = serde_json::from_str(&bytes).map_err(io::Error::other)?;
        crate::agent_proto::ValidatedRelativePath::parse(&recovery.rel)?;
        if recovery.pair != key.pair_id
            || recovery.lock != key.lock_id
            || path(key, &recovery.rel)? != entry.path()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pending merge belongs to another state",
            ));
        }
        relatives.insert(recovery.rel);
        if let Some(sibling) = recovery.sibling {
            crate::agent_proto::ValidatedRelativePath::parse(&sibling)?;
            relatives.insert(sibling);
        }
    }
    Ok(relatives.into_iter().collect())
}

/// An interrupted target-side archive has its durable intent in the same
/// run. Restore it create-only before validating a retry's original text.
pub(super) fn recover_original(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    path: &str,
    original: OriginalContent<'_>,
    recovery: &Recovery,
    side: PairSide,
    key: &StateKey,
    cross_mounts: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let done = if side == PairSide::A {
        recovery.done_a
    } else {
        recovery.done_b
    };
    if done || original.bytes.is_none() || backend.try_exists(path)? {
        return Ok(());
    }
    let side = VersionSide {
        side,
        backend,
        root,
    };
    let entries =
        super::versions::list_versions(&key.pair_id, std::slice::from_ref(&side), cancel)?;
    for entry in entries.into_iter().filter(|entry| {
        entry.run_id == recovery.run
            && entry.rel == rel
            && entry.side == Some(side.side)
            && entry.reason == Some(VersionReason::Resolved)
            && entry.store == super::versions::VersionStore::SyncRoot
    }) {
        let current = capture(
            backend,
            &entry.stored_path,
            ExpectedFile::Unknown,
            "merge recovery version",
        )?;
        let meta = current.regular("merge recovery version")?;
        let mut reader =
            crate::vfs::open_read_regular(backend, &entry.stored_path, meta.id.as_deref())?;
        let bytes = original.bytes.unwrap_or_default();
        let mut compared = Compared::new(bytes, bytes);
        super::transfer_stream::stream(&mut *reader, &mut compared, cancel, None, 0, |_| {})?;
        drop(reader);
        if !compared.first_matches() {
            continue;
        }
        revalidate(
            backend,
            &entry.stored_path,
            &current,
            "merge recovery version",
        )?;
        super::apply_boundary::guard(backend, root, rel, cross_mounts)?;
        backend.rename_no_replace(&entry.stored_path, path)?;
        return super::apply_stage::require_durable(super::apply_stage::namespace(backend, path)?);
    }
    // Retention may have removed a target archive since the crash. The
    // durable private inputs still contain the exact authorized original.
    let bytes = original
        .bytes
        .ok_or_else(|| drift("merge original input is absent"))?;
    let expected = if side.side == PairSide::A {
        &recovery.original_a
    } else {
        &recovery.original_b
    };
    if expected.as_deref() != Some(super::merge_precheck::digest(bytes).as_str()) {
        return Err(drift("merge recovery original input changed"));
    }
    super::apply_boundary::guard(backend, root, rel, cross_mounts)?;
    super::apply_boundary::target(backend, root, rel, Some(bytes.len() as u64))?;
    let absent = capture(
        backend,
        path,
        ExpectedFile::Missing,
        "merge recovery original",
    )?;
    let stage = super::apply_stage::stage_bytes(
        backend,
        path,
        &absent,
        bytes,
        original
            .signature
            .map_or(recovery.started_ms, |sig| sig.mtime_ms),
        cancel,
    )?;
    let result = stage.publish(path, &absent, true, cancel)?;
    super::apply_stage::require_durable(result.durable)
}
