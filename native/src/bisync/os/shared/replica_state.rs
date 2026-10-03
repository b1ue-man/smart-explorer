//! Where the stored state of a pair lives (V3, B01, Y150): one baseline per
//! (pair, owner, replica A, replica B) under `sync/pairs/<pair_id>/`, the
//! pair-wide `baseline_<pair>.sebl` of the time before RV1 for the legacy key.
//! Results of conflict resolutions and single actions are merged into the
//! state while the pair's lock is held (Y45); deleting a job can remove
//! everything it stored.
use std::io;
use std::path::{Path, PathBuf};

use super::pair_lock::PairLock;
use super::persistence::baseline_path;
use super::run_types::{ReplicaRef, StateKey, StateOwner};
use super::state_store::SyncStateStore;
use super::types::Sig;

/// Folder of the per-pair state folders below the sync data directory.
const PAIRS_DIR: &str = "pairs";
const BASELINE_EXTENSION: &str = "sebl";

/// The baseline file of one stored state.
pub fn baseline_file(key: &StateKey) -> io::Result<PathBuf> {
    validate_hex(&key.pair_id, "pair id")?;
    if key.is_legacy() {
        return Ok(baseline_path(&key.pair_id));
    }
    Ok(pair_dir(&key.pair_id).join(format!(
        "{}.{}.{BASELINE_EXTENSION}",
        owner_token(&key.owner)?,
        replica_token(&key.replica_a, &key.replica_b)
    )))
}

/// Merges finished results into the stored state while the caller holds the
/// pair's lock (conflict resolution, single actions, restores): each entry
/// replaces the stored one, `(None, None)` removes it. Concurrent runs of the
/// pair cannot interleave, and nothing a run recorded meanwhile is lost.
pub fn merge_baseline_entries(
    lock: &PairLock,
    key: &StateKey,
    entries: &[(String, (Option<Sig>, Option<Sig>))],
) -> io::Result<()> {
    merge_with_keys(lock, key, entries, super::KeyPolicy::default())
}

pub(super) fn merge_with_keys(
    lock: &PairLock, key: &StateKey,
    entries: &[(String, (Option<Sig>, Option<Sig>))], keys: super::KeyPolicy,
) -> io::Result<()> {
    if lock.id() != key.lock_id.to_ascii_lowercase() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the held lock belongs to another sync pair",
        ));
    }
    let (mut journal, mut records, dirs) = super::checkpoint_journal::Journal::load(key, keys)?;
    let mut dirs = dirs.unwrap_or_default();
    let frame = super::checkpoint_journal::Frame { fold_case: keys.fold_case, records: entries.to_vec(),
        ..super::checkpoint_journal::Frame::default() };
    frame.ensure_fits(&records, &dirs, super::checkpoint_journal::dir_bytes(&dirs),
        super::SyncLimits::for_memory(crate::transfer::physical_memory()))?;
    journal.append(&frame)?;
    frame.apply(&mut records, &mut dirs)?;
    journal.compact(key, &records, &dirs)
}

/// Removes every stored state a job owns, for all its pairs (offered when a
/// job is deleted or retargeted). Versions: `versions::remove_versions`.
pub fn forget_job_state(job_id: &str) -> io::Result<()> {
    let prefix = format!("{}.", owner_token(&StateOwner::Job(job_id.to_string()))?);
    let root = crate::support_dirs::sync_data_dir().join(PAIRS_DIR);
    let pairs = match std::fs::read_dir(&root) {
        Ok(pairs) => pairs,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return SyncStateStore::open_default().and_then(|mut store| store.forget_owner(&owner_token(&StateOwner::Job(job_id.to_string()))
                .map_err(|_| rusqlite::Error::InvalidQuery)?)).map_err(index_error);
        }
        Err(error) => return Err(error),
    };
    for pair in pairs {
        let pair = pair?;
        if !pair.file_type()?.is_dir() {
            continue;
        }
        remove_files(&pair.path(), |name| name.starts_with(&prefix))?;
        remove_dir_if_empty(&pair.path())?;
    }
    SyncStateStore::open_default().and_then(|mut store| store.forget_owner(
        &owner_token(&StateOwner::Job(job_id.to_string())).map_err(|_| rusqlite::Error::InvalidQuery)?)).map_err(index_error)
}

/// Removes the stored state of one pair for every owner: the per-pair folder,
/// the pair-wide baseline and the incremental index.
pub fn forget_pair_state(pair_id: &str) -> io::Result<()> {
    validate_hex(pair_id, "pair id")?;
    match std::fs::remove_file(baseline_path(pair_id)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let dir = pair_dir(pair_id);
    match std::fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            remove_files(&dir, |_| true)?;
            remove_dir_if_empty(&dir)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    SyncStateStore::open_default()
        .and_then(|mut store| store.forget_pair_family(pair_id))
        .map_err(index_error)
}

fn index_error(error: rusqlite::Error) -> io::Error {
    io::Error::other(format!("incremental sync index: {error}"))
}

pub(super) fn pair_dir(pair_id: &str) -> PathBuf {
    crate::support_dirs::sync_data_dir()
        .join(PAIRS_DIR)
        .join(pair_id)
}

pub(super) fn owner_token(owner: &StateOwner) -> io::Result<String> {
    match owner {
        StateOwner::AdHoc => Ok("adhoc".to_string()),
        StateOwner::Job(id)
            if !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            Ok(format!("job-{id}"))
        }
        StateOwner::Job(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sync job id is not safe for a state file name",
        )),
    }
}

pub(super) fn owner_from_token(token: &str) -> io::Result<StateOwner> {
    let owner = if token == "adhoc" { StateOwner::AdHoc } else {
        StateOwner::Job(token.strip_prefix("job-").ok_or_else(|| io::Error::new(
            io::ErrorKind::InvalidData, "invalid pending state owner"))?.to_string())
    };
    if owner_token(&owner)? != token {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid pending state owner"));
    }
    Ok(owner)
}

/// Only owners of these exact ordered endpoints and physical replicas are
/// relevant to a pending merge. Their state entries are never imported.
pub(super) fn pending_owner_keys(key: &StateKey) -> io::Result<Vec<StateKey>> {
    let mut owners = std::collections::BTreeSet::from([owner_token(&key.owner)?, "adhoc".to_string()]);
    let suffix = format!(".{}.merge-", replica_token(&key.replica_a, &key.replica_b));
    let entries = match std::fs::read_dir(pair_dir(&key.pair_id)) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut count = 0u64;
    for entry in entries.into_iter().flatten() {
        count = count.saturating_add(1);
        if count > limits.state_entries { return Err(io::Error::other("pending owner collection exceeds its budget")); }
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue; };
        if name.ends_with(".json") {
            if let Some((owner, _)) = name.split_once(&suffix) {
                owner_from_token(owner)?;
                owners.insert(owner.to_string());
            }
        }
    }
    let mut keys = owners.into_iter().map(|owner| Ok(StateKey {
        owner: owner_from_token(&owner)?, ..key.clone()
    })).collect::<io::Result<Vec<_>>>()?;
    if !key.is_legacy() {
        // The old pair-wide adhoc intent has no marker tokens. Its inputs
        // remain protected without adopting its baseline into this replica.
        keys.push(StateKey { owner: StateOwner::AdHoc, replica_a: ReplicaRef::Unknown,
            replica_b: ReplicaRef::Unknown, ..key.clone() });
    }
    Ok(keys)
}

/// Short, stable token of both replica identities (part of the file name).
pub(super) fn replica_token(a: &ReplicaRef, b: &ReplicaRef) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    feed(&mut hash, b"smart-explorer/bisync-replicas/v1");
    feed(&mut hash, b"a");
    feed_replica(&mut hash, a);
    feed(&mut hash, b"b");
    feed_replica(&mut hash, b);
    format!("{hash:016x}")
}

/// The optional index shares exactly the baseline's owner and replicas.
pub(super) fn index_id(key: &StateKey) -> io::Result<String> {
    if key.is_legacy() {
        return Ok(key.pair_id.clone());
    }
    Ok(format!("{}:{}:{}", key.pair_id, owner_token(&key.owner)?,
        replica_token(&key.replica_a, &key.replica_b)))
}

fn feed_replica(hash: &mut u64, replica: &ReplicaRef) {
    let (tag, value) = match replica {
        ReplicaRef::Marker(id) => ("m", id.as_str()),
        ReplicaRef::Volume(volume) => ("v", volume.as_str()),
        ReplicaRef::Unknown => ("u", ""),
    };
    feed(hash, tag.as_bytes());
    feed(hash, &(value.len() as u64).to_be_bytes());
    feed(hash, value.as_bytes());
}

fn feed(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

fn validate_hex(value: &str, label: &str) -> io::Result<()> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid sync {label}"),
        ));
    }
    Ok(())
}

/// Removes the regular files of `dir` whose names match; links and folders
/// are never followed or removed.
fn remove_files(dir: &Path, matches: impl Fn(&str) -> bool) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if entry.file_name().to_str().is_some_and(&matches) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn remove_dir_if_empty(dir: &Path) -> io::Result<()> {
    if std::fs::read_dir(dir)?.next().is_some() {
        return Ok(());
    }
    match std::fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "replica_state_tests.rs"]
mod tests;
