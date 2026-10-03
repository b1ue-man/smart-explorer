//! Private durable original texts and exact merge choice for process restarts.
use std::io::{self, Read};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::merge_recovery::Recovery;
use super::run_types::StateKey;
use super::types::{Conflict, Sig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordedMergeChoice { Write, KeepBoth { keep_a: bool } }
#[derive(Clone, Debug)]
pub struct PendingMerge {
    pub conflict: Conflict,
    pub original_a: Option<Vec<u8>>,
    pub original_b: Option<Vec<u8>>,
    pub merged: Vec<u8>,
    pub choice: RecordedMergeChoice,
    pub confirmed_a: Option<Sig>,
    pub confirmed_b: Option<Sig>,
}
#[derive(Serialize, Deserialize)]
struct InputRecord {
    format: u8, pair: String, lock: String, rel: String,
    a: Option<Sig>, b: Option<Sig>, length_a: Option<u64>, length_b: Option<u64>, length_merged: u64,
}
fn dir(key: &StateKey, rel: &str) -> io::Result<PathBuf> {
    Ok(super::replica_state::baseline_file(key)?.with_extension(
        format!("merge-{}.inputs", super::version_manifest::token(rel))))
}
fn maximum() -> u64 { super::SyncLimits::for_memory(crate::transfer::physical_memory()).walk_text_bytes }
pub(super) fn save(key: &StateKey, recovery: &Recovery,
    a: super::merge_recorded::OriginalContent<'_>, b: super::merge_recorded::OriginalContent<'_>, merged: &[u8],
) -> io::Result<()> {
    let total = a.bytes.map_or(0, |bytes| bytes.len() as u64)
        .saturating_add(b.bytes.map_or(0, |bytes| bytes.len() as u64)).saturating_add(merged.len() as u64);
    if total > maximum() { return Err(io::Error::other("merge inputs exceed their memory budget")); }
    let dir = dir(key, &recovery.rel)?;
    crate::support_dirs::ensure_private_dir(&dir)?;
    for (name, bytes) in [("original-a",a.bytes),("original-b",b.bytes),("merged",Some(merged))] {
        if let Some(bytes) = bytes { crate::support_dirs::write_private_atomic(&dir.join(name), bytes)?; }
    }
    let record = InputRecord { format: 1, pair: key.pair_id.clone(), lock: key.lock_id.clone(), rel: recovery.rel.clone(),
        a: a.signature, b: b.signature, length_a: a.bytes.map(|bytes| bytes.len() as u64),
        length_b: b.bytes.map(|bytes| bytes.len() as u64), length_merged: merged.len() as u64 };
    let bytes = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    crate::support_dirs::write_private_atomic(&dir.join("inputs.json"), &bytes)
}
fn read(dir: &std::path::Path, name: &str, length: u64, digest: &str) -> io::Result<Vec<u8>> {
    if length > maximum() { return Err(io::Error::other("merge input exceeds its memory budget")); }
    let mut file = crate::support_dirs::open_private_file(&dir.join(name))?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != length {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "merge input file changed"));
    }
    let mut bytes = Vec::new(); file.by_ref().take(length.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length || format!("{:x}",md5::compute(&bytes)) != digest {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "merge input content changed"));
    }
    Ok(bytes)
}
pub(super) fn load(key: &StateKey, recovery: &Recovery) -> io::Result<PendingMerge> {
    let dir = dir(key, &recovery.rel)?;
    let text = crate::support_dirs::read_private_text(&dir.join("inputs.json"), 256 * 1024)?;
    let record: InputRecord = serde_json::from_str(&text).map_err(io::Error::other)?;
    if record.format != 1 || record.pair != key.pair_id || record.lock != key.lock_id || record.rel != recovery.rel
        || record.a.is_some() != record.length_a.is_some() || record.b.is_some() != record.length_b.is_some()
        || record.length_a.unwrap_or(0).saturating_add(record.length_b.unwrap_or(0)).saturating_add(record.length_merged) > maximum() {
        return Err(io::Error::new(io::ErrorKind::InvalidData,"merge inputs belong to another state"));
    }
    let original_a = record.length_a.zip(recovery.original_a.as_deref())
        .map(|(length,digest)| read(&dir,"original-a",length,digest)).transpose()?;
    let original_b = record.length_b.zip(recovery.original_b.as_deref())
        .map(|(length,digest)| read(&dir,"original-b",length,digest)).transpose()?;
    if original_a.is_some() != record.a.is_some() || original_b.is_some() != record.b.is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidData,"merge input presence changed"));
    }
    let merged = read(&dir,"merged",record.length_merged,&recovery.merged)?;
    let choice = match recovery.kind.as_str() {
        "write" => RecordedMergeChoice::Write,
        "keep-a" => RecordedMergeChoice::KeepBoth { keep_a: true },
        "keep-b" => RecordedMergeChoice::KeepBoth { keep_a: false },
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData,"invalid recorded merge choice")),
    };
    Ok(PendingMerge { conflict: Conflict { rel: recovery.rel.clone(), a: record.a, b: record.b, duplicates: None },
        original_a, original_b, merged, choice,
        confirmed_a: if recovery.done_a { recovery.a } else { None },
        confirmed_b: if recovery.done_b { recovery.b } else { None } })
}
pub(super) fn remove(key: &StateKey, rel: &str) -> io::Result<()> {
    let dir = dir(key, rel)?;
    for name in ["original-a","original-b","merged","inputs.json"] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}, Err(error) if error.kind() == io::ErrorKind::NotFound => {}, Err(error) => return Err(error),
        }
    }
    match std::fs::remove_dir(&dir) {
        Ok(()) => {}, Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()), Err(error) => return Err(error),
    }
    let path = dir.to_str().ok_or_else(|| io::Error::other("merge inputs path is not Unicode"))?;
    super::apply_stage::require_durable(super::apply_stage::namespace(&crate::vfs::LocalBackend::new(path),path)?)
}
