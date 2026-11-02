//! Pure Share path parsing. Operation paths are names, not user-input labels.

/// The clean split of operation paths lives in the storage layer (D13).
pub(super) use crate::vfs::split_share_path as split_clean;

pub(super) fn join_under(root: &str, rest: &[String]) -> String {
    let root = root.replace('\\', "/");
    if rest.is_empty() {
        return norm_root(&root);
    }
    let base = norm_root(&root);
    format!("{}/{}", base.trim_end_matches('/'), rest.join("/"))
}

/// Normalize configured export/connection roots, never operation-path names.
pub(super) fn norm_root(root: &str) -> String {
    let root = root.trim().replace('\\', "/");
    if root.is_empty() {
        return "/".to_string();
    }
    let bytes = root.as_bytes();
    if bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return format!("{root}/");
    }
    if bytes.len() == 3 && bytes[1] == b':' && bytes[2] == b'/'
        && bytes[0].is_ascii_alphabetic()
    {
        return root;
    }
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }
}
