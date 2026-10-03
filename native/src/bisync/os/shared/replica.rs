//! Replica identity at the shared Backend boundary, never through local path
//! APIs for remote roots. Missing and unavailable are different observations.
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use super::incremental::SyncEndpoints;
use super::paths::REPLICA_MARKER_NAME;
use super::run_types::{ReplicaRef, RunBlock, RunSettings, StateKey};
use super::state_metadata::{load_history, now_ms, PairHistory};
use super::types::PairSide;
use crate::vfs::{Backend, StageDurability, StageFinish};

const MARKER_BYTES: u64 = 4096;

#[derive(Serialize, Deserialize)]
struct Marker {
    replica_id: String,
    created_ms: i64,
    pair_hint: String,
}

#[derive(Clone, Copy)]
enum MarkerRead { Present, Missing, Unavailable }

pub(super) struct Replicas {
    pub key: StateKey,
    pub history: Option<PairHistory>,
    pub blocked: Option<RunBlock>,
    pub changed: bool,
    /// A marker just created on the same known volume upgrades its identity;
    /// this is not a rotation and its owner's completed basis is carried over.
    pub upgraded_from: Option<StateKey>,
}

pub(super) fn identify(
    endpoints: SyncEndpoints<'_>,
    settings: &RunSettings,
    write_markers: bool,
) -> io::Result<Replicas> {
    let mut key = StateKey::legacy(
        &super::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b),
        &super::pair_lock_id(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b),
    );
    key.owner = settings.owner.clone();
    let history = load_history(&key)?;
    let mut blocked = None;
    let mut upgrade_only = true;
    let mut observed = Vec::with_capacity(2);
    for (side, backend, root) in [
        (PairSide::A, endpoints.a, endpoints.root_a),
        (PairSide::B, endpoints.b, endpoints.root_b),
    ] {
        let (identity, status) = observe(backend, root)?;
        let previous = history.as_ref().map(|history| history.replica(side));
        let missing = RunBlock::ReplicaMissing { side };
        if matches!(status, MarkerRead::Missing)
            && matches!(previous, Some(ReplicaRef::Marker(_)))
            && !settings.confirmed.iter().any(|confirmation| confirmation.covers(&missing))
        {
            blocked.get_or_insert(missing);
        }
        observed.push((side, backend, root, identity, status));
    }
    // Observe both guards before even creating our own marker on either side.
    for (side, backend, root, mut identity, status) in observed {
        let previous = history.as_ref().map(|history| history.replica(side));
        let before = identity.clone();
        let mut created = false;
        if blocked.is_none() && matches!(status, MarkerRead::Missing) && write_markers {
            match create_marker(backend, root, &key.pair_id) {
                Ok(replica) => { identity = replica; created = true; }
                // Read-only sources and providers without atomic create keep
                // their stable volume identity (or Unknown). No user capability
                // is narrowed by the inability to store our own marker.
                Err(error) if marker_unavailable(&error) => {}
                Err(error) => return Err(error),
            }
        }
        if matches!(status, MarkerRead::Unavailable) || identity == ReplicaRef::Unknown {
            // An unreadable marker cannot prove that a formerly known replica
            // disappeared. Keep its identity until an actual observation exists.
            if let Some(previous) = previous {
                identity = previous.clone();
            }
        }
        if previous.is_some_and(|previous| *previous != identity) {
            upgrade_only &= created && previous.is_some_and(|previous| {
                previous == &before || *previous == ReplicaRef::Unknown
            });
        }
        match side {
            PairSide::A => key.replica_a = identity,
            PairSide::B => key.replica_b = identity,
        }
    }
    let changed = history.as_ref().is_some_and(|history| !history.matches(&key));
    let upgraded_from = history.as_ref().filter(|_| changed && upgrade_only).map(|history| StateKey {
        replica_a: history.replica_a.clone(), replica_b: history.replica_b.clone(), ..key.clone()
    });
    Ok(Replicas { key, history, blocked, changed, upgraded_from })
}

fn observe(backend: &dyn Backend, root: &str) -> io::Result<(ReplicaRef, MarkerRead)> {
    let path = crate::vfs::sync_child_path(backend, root, REPLICA_MARKER_NAME)?;
    let status = match backend.stat(&path) {
        Ok(meta) if meta.is_dir || meta.is_symlink || meta.special || meta.size > MARKER_BYTES => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Sync-Markierung ist keine gültige Datei"));
        }
        Ok(_) => {
            let mut bytes = Vec::new();
            match crate::vfs::open_read_regular(backend, &path, None)
                .and_then(|reader| reader.take(MARKER_BYTES + 1).read_to_end(&mut bytes))
            {
                Ok(_) if bytes.len() as u64 <= MARKER_BYTES => {
                    let marker: Marker = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                    if marker.replica_id.len() != 32
                        || !marker.replica_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Ungültige Replika-ID"));
                    }
                    return Ok((ReplicaRef::Marker(marker.replica_id.to_ascii_lowercase()), MarkerRead::Present));
                }
                Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "Sync-Markierung ist zu groß")),
                Err(error) if marker_unavailable(&error) => MarkerRead::Unavailable,
                Err(error) => return Err(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => MarkerRead::Missing,
        Err(error) if marker_unavailable(&error) => MarkerRead::Unavailable,
        Err(error) => return Err(error),
    };
    let volume = match crate::vfs::volume_identity(backend, root) {
        Ok(volume) => volume.map(|volume| ReplicaRef::Volume(volume.key())).unwrap_or(ReplicaRef::Unknown),
        Err(error) if marker_unavailable(&error) => ReplicaRef::Unknown,
        Err(error) => return Err(error),
    };
    Ok((volume, status))
}

fn create_marker(backend: &dyn Backend, root: &str, pair: &str) -> io::Result<ReplicaRef> {
    // Never create an unavailable/unmounted root merely to write a marker.
    let root_meta = backend.stat(root)?;
    if !root_meta.is_dir || root_meta.is_symlink {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "Sync-Markierung kann nicht auf diesem Wurzeltyp gespeichert werden"));
    }
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let replica_id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let marker = Marker { replica_id: replica_id.clone(), created_ms: now_ms(), pair_hint: pair.to_string() };
    let path = crate::vfs::sync_child_path(backend, root, REPLICA_MARKER_NAME)?;
    let stage = crate::vfs::unique_staging_path(backend, &path, "sync-replica")?;
    let result = (|| {
        let bytes = serde_json::to_vec(&marker).map_err(io::Error::other)?;
        let mut writer = backend.open_write_copy_stage_sized(&stage, bytes.len() as u64)?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        drop(writer);
        crate::vfs::finish_stage(backend, &stage, StageFinish {
            durability: StageDurability::Now, ..StageFinish::default()
        })?;
        crate::vfs::promote_staged_create(backend, &stage, &path)?;
        crate::vfs::sync_filesystem(backend, root)?;
        Ok(ReplicaRef::Marker(replica_id))
    })();
    if result.is_err() {
        let _ = backend.discard_copy_stage(&stage);
    }
    match result {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => observe(backend, root).map(|(id, _)| id),
        result => result,
    }
}

fn marker_unavailable(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
        | io::ErrorKind::Unsupported)
}
