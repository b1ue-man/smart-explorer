//! Pure Share path parsing. Operation paths are names, not user-input labels.
use std::io;

use super::core::eio;

pub(super) fn split_clean(path: &str) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    // Leading/trailing whitespace can belong to a filename. Reject invalid
    // components without silently retargeting a valid operation's spelling.
    for part in path.trim_matches('/').split('/') {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." || part.contains('\\') || part.contains('\0') {
            return Err(eio("Ungueltiger Pfad"));
        }
        out.push(part.to_string());
    }
    Ok(out)
}

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
