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

pub(super) fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    }
}

pub(super) fn rel_of(path: &str, root: &str) -> String {
    let r = root.trim_end_matches('/');
    path.strip_prefix(r)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| path.trim_start_matches('/').to_string())
}

pub(super) fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    t.rfind('/')
        .map(|i| if i == 0 { "/".into() } else { t[..i].into() })
}
