//! Immutable, bounded version records. Intent records survive an interrupted rename.
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;
use serde::{Deserialize, Serialize};
use crate::vfs::{Backend, StageDurability, StageFinish};
use super::paths::{join, parent_of};
use super::types::{PairSide, Sig};
use super::versions::{RunVersions, VersionEntry, VersionReason, VersionSide, VersionStore};
use super::transfer_stream::check;

const MAX_MANIFEST: u64 = 256 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub format: u32,
    pub pair: String,
    pub lock: String,
    pub backend: String,
    pub root: String,
    pub replica: String,
    pub side: String,
    pub rel: String,
    pub run: String,
    pub preserved_ms: i64,
    pub reason: String,
    pub original: Sig,
    pub data_sig: Sig,
    pub data_id: Option<String>,
    pub digest: Option<String>,
    pub size: u64,
    pub data: String,
    pub job: Option<String>,
    pub job_name: String,
}
impl Manifest {
    pub(super) fn new(versions: &RunVersions, side: &VersionSide<'_>, rel: &str,
        reason: VersionReason, signature: Sig, data: String, replica: String) -> Self {
        Self { format: 1, pair: versions.context().pair_id.clone(), lock: versions.lock_id(),
            backend: side.backend.state_identity(), root: side.root.to_string(), replica,
            side: side.side.as_str().to_string(), rel: rel.to_string(), run: versions.run_id().to_string(),
            preserved_ms: super::versions::now_ms(), reason: reason_code(reason).to_string(),
            original: signature, data_sig: signature, data_id: None, digest: None, size: signature.size, data,
            job: match &versions.context().owner { super::run_types::StateOwner::Job(id) => Some(id.clone()),
                super::run_types::StateOwner::AdHoc => None },
            job_name: versions.context().job_name.clone() }
    }
    pub(super) fn entry(&self, store: VersionStore) -> io::Result<VersionEntry> {
        Ok(VersionEntry { side: Some(PairSide::parse(&self.side).ok_or_else(|| invalid("invalid version side"))?),
            rel: self.rel.clone(), run_id: self.run.clone(), preserved_ms: self.preserved_ms,
            reason: Some(reason_parse(&self.reason)?), size: self.size, mtime_ms: self.original.mtime_ms,
            store, stored_path: self.data.clone(), job_id: self.job.clone() })
    }
    pub(super) fn belongs(&self, pair: &str, side: &VersionSide<'_>) -> bool {
        if self.format != 1 || self.root != side.root || self.side != side.side.as_str() { return false; }
        let backend = side.backend.state_identity();
        (self.pair == pair && self.backend == backend)
            || super::backend_identity_state::version_matches(pair, &self.pair, &self.backend,
                &self.root, if side.side == PairSide::A { 0 } else { 1 }, &backend).unwrap_or(false)
    }
}
pub(super) fn write(backend: &dyn Backend, path: &str, manifest: &Manifest) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_MANIFEST { return Err(invalid("version manifest exceeds its budget")); }
    let stage = crate::vfs::unique_staging_path(backend, path, "version-index")?;
    let result = (|| {
        let mut writer = backend.open_write_copy_stage_sized(&stage, bytes.len() as u64)?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        drop(writer);
        crate::vfs::finish_stage(backend, &stage, StageFinish {
            durability: StageDurability::Now, mode: Some(0o600), ..StageFinish::default()
        })?;
        crate::vfs::promote_staged_create(backend, &stage, path)?;
        super::apply_stage::require_durable(super::apply_stage::namespace(backend, path)?)?;
        Ok(())
    })();
    if result.is_err() { let _ = backend.discard_copy_stage(&stage); }
    result
}
pub(super) fn read(backend: &dyn Backend, path: &str, cancel: &AtomicBool) -> io::Result<Manifest> {
    check(cancel)?;
    let observed = super::apply_guard::capture(backend, path,
        super::apply_guard::ExpectedFile::Unknown, "version record")?;
    if observed.metadata.is_none() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "version record is absent"));
    }
    let meta = observed.regular("version record")?;
    if meta.is_dir || meta.is_symlink || meta.special || meta.size > MAX_MANIFEST {
        return Err(invalid("version record is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    crate::vfs::open_read_regular(backend, path, meta.id.as_deref())?
        .take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST { return Err(invalid("version record is too large")); }
    super::apply_guard::revalidate(backend, path, &observed, "version record")?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    super::paths::validate_child_name(&manifest.run)?;
    crate::agent_proto::ValidatedRelativePath::parse(&manifest.rel)?;
    let parent = parent_of(path).ok_or_else(|| invalid("version record has no parent"))?;
    if manifest.data != join(&parent, "data") || manifest.format != 1 {
        return Err(invalid("version record redirects outside its entry"));
    }
    Ok(manifest)
}
pub(super) fn ensure_dirs(backend: &dyn Backend, root: &str, relative: &str) -> io::Result<String> {
    let root_meta = backend.stat(root)?;
    if !root_meta.is_dir || root_meta.is_symlink || root_meta.special {
        return Err(invalid("version root is not a real directory"));
    }
    let mut path = root.to_string();
    for name in relative.split('/') {
        super::paths::validate_child_name(name)?;
        path = join(&path, name);
        match backend.stat(&path) {
            Ok(meta) if meta.is_dir && !meta.is_symlink && !meta.special => {},
            Ok(_) => return Err(invalid("version path has a link or non-directory")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                backend.mkdir_all(&path)?;
                let meta = backend.stat(&path)?;
                if !meta.is_dir || meta.is_symlink || meta.special { return Err(invalid("version directory changed")); }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(path)
}
pub(super) fn token(value: &str) -> String {
    format!("{:x}", md5::compute(value.as_bytes()))
}
pub(super) fn random() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(super) fn reason_code(reason: VersionReason) -> &'static str {
    match reason { VersionReason::Replaced => "replaced", VersionReason::Deleted => "deleted",
        VersionReason::Resolved => "resolved", VersionReason::Restored => "restored" }
}
fn reason_parse(reason: &str) -> io::Result<VersionReason> {
    match reason { "replaced" => Ok(VersionReason::Replaced), "deleted" => Ok(VersionReason::Deleted),
        "resolved" => Ok(VersionReason::Resolved), "restored" => Ok(VersionReason::Restored),
        _ => Err(invalid("invalid version reason")) }
}
pub(super) fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// Public version operations accept only persisted pair identifiers.
pub(super) fn validate_pair(pair: &str) -> io::Result<()> {
    if pair.is_empty() || pair.len() > 256 || matches!(pair, "." | "..")
        || !pair.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) {
        return Err(invalid("invalid version pair identity"));
    }
    Ok(())
}
