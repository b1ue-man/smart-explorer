//! The file signature and the reserved names of the sync engine. They live in
//! the storage layer (plan docs/plaene/2026-11-01-idealzustand, D13) because
//! backends, walks and analyses below the sync engine use them; `bisync`
//! re-exports them.

/// Size + mtime (+ optional content hash) signature of one file on one side.
/// `hash` is 0 when this side wasn't hashed (see `HashMode`/`hash_mode`); when
/// non-zero it's the MD5-derived content key and takes priority in compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sig {
    pub size: u64,
    pub mtime_ms: i64,
    pub hash: u64,
}

/// Marker file with the replica ID at every sync root (V3, B01).
pub const REPLICA_MARKER_NAME: &str = ".se-sync-replica";
/// Hidden folder at a sync root holding the versions of replaced and deleted
/// files (FS5).
pub const VERSIONS_DIR_NAME: &str = ".se-versions";

/// Names the sync engine itself creates inside sync roots. Walks, watchers,
/// duplicate searches and Share listings treat them as the engine's own
/// entries (`OmissionKind::OwnFile`) at any depth: a sync root nested inside
/// another job's root carries its own marker and versions.
pub fn is_engine_name(name: &str) -> bool {
    name == REPLICA_MARKER_NAME || name == VERSIONS_DIR_NAME
}
