//! The namespace of a Share peer: the clean split of operation paths and the
//! synthetic mountpoint containers. Share serves it and the agent backend
//! reaches it for peers, so it lives in the storage layer (plan
//! docs/plaene/2026-11-01-idealzustand, D13); `share` re-exports it.
use std::io;

/// The container of the peer's saved connections.
pub const CONNECTIONS_ROOT: &str = "/Verbindungen";

/// The components of a Share operation path. Leading/trailing whitespace can
/// belong to a filename; `.`, `..`, backslashes and NUL are refused instead
/// of silently retargeting a valid operation's spelling.
pub fn split_clean(path: &str) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for part in path.trim_matches('/').split('/') {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." || part.contains('\\') || part.contains('\0') {
            return Err(io::Error::other("Ungueltiger Pfad"));
        }
        out.push(part.to_string());
    }
    Ok(out)
}

/// Whether `path` is one of the synthetic mountpoint containers (`/` and
/// `/Verbindungen`), which hold no files of their own; invalid paths count
/// as such, since nothing can be written there either.
pub fn is_synthetic_container(path: &str) -> bool {
    match normalize(path) {
        Ok(normalized) => normalized == "/" || normalized == CONNECTIONS_ROOT,
        Err(_) => true,
    }
}

/// `/` or `/<components>` of a Share operation path.
pub fn normalize(path: &str) -> io::Result<String> {
    let parts = split_clean(path)?;
    if parts.is_empty() {
        Ok("/".into())
    } else {
        Ok(format!("/{}", parts.join("/")))
    }
}
