/// The reserved names live in the storage layer (D13).
pub use crate::vfs::{is_engine_name, REPLICA_MARKER_NAME, VERSIONS_DIR_NAME};

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
