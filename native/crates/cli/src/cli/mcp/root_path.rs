//! The working root of an MCP session (`root` tool, plan 2026-11-27 section
//! C): which kind of location it is, and how a target of a file tool is
//! placed under it. A relative target (`src/main.rs`, `.`, empty) is a path
//! below the root; `..` may climb inside it but never above it. A target
//! that spells the root itself or a path below it is placed the same way, so
//! it reuses the root's open connection. Every other absolute target stays
//! what it is.
use crate::creds::Protocol;
use std::sync::Arc;

/// Which connection a working root opens; decides how paths below it reuse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RootKind {
    /// A local path (cheap to open; every call resolves it again).
    Local,
    /// A UNC path (`\\server\share`), with the login of a saved share when one covers it.
    Unc,
    /// A saved SFTP, FTP, FTPS or WebDAV connection (one login per session).
    Login(Protocol),
    /// A saved SMB connection.
    Smb,
    /// A saved Windows share (`net use` lease).
    WindowsShare,
    /// Google Drive.
    Drive,
    /// A Share peer (Direct or a room device).
    Peer,
    /// The cleartext view of an unlocked vault, or a vault's storage.
    Vault,
    /// Another remote URL (not saved; its resolution reports why).
    Url,
}

impl RootKind {
    pub(super) fn label(self, spec: &str) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Unc => "unc",
            Self::Login(protocol) => protocol.as_str(),
            Self::Smb => "smb",
            Self::WindowsShare => "windows-share",
            Self::Drive => "gdrive",
            Self::Peer if spec.starts_with("share://room/") => "share-room",
            Self::Peer => "share-direct",
            Self::Vault => "vault",
            Self::Url => "remote",
        }
    }

    /// Paths below a root of this kind use the root's own connection (the
    /// resolution of every other kind keeps its connection itself or is
    /// cheap: local paths, saved logins, Share mounts).
    pub(super) fn shares_root_connection(self) -> bool {
        matches!(
            self,
            Self::Unc | Self::Smb | Self::WindowsShare | Self::Drive | Self::Vault
        )
    }
}

/// The working root set with `root`.
pub(super) struct WorkRoot {
    /// The absolute target, as given (relative ones resolved against the
    /// previous root).
    pub(super) spec: String,
    pub(super) kind: RootKind,
    /// The path the resolution reported for it (for a Share peer the path in
    /// the Share namespace, `/<mount>/…`).
    pub(super) path: String,
}

/// Where a target of a file tool lies.
pub(super) enum Place {
    /// Below the working root: the normalized relative path ("" = the root).
    Under(Arc<WorkRoot>, String),
    /// An absolute target outside of it, or no root is set.
    Absolute(String),
}

pub(super) fn place(spec: &str, root: Option<Arc<WorkRoot>>) -> Result<Place, String> {
    let spec = spec.trim();
    let Some(root) = root else {
        return Ok(Place::Absolute(spec.to_string()));
    };
    if is_relative(spec) {
        let rel = normalize_relative(spec, &root.spec)?;
        return Ok(Place::Under(root, rel));
    }
    // The root's own spelling, or a path below it, as long as it stays inside.
    if let Some(rest) = below(spec, &root.spec) {
        if let Ok(rel) = normalize_relative(rest, &root.spec) {
            return Ok(Place::Under(root, rel));
        }
    }
    Ok(Place::Absolute(spec.to_string()))
}

/// Neither a saved remote, an endpoint nor an absolute local or UNC path.
pub(super) fn is_relative(spec: &str) -> bool {
    !(spec.starts_with('@') || spec.contains("://") || is_absolute_local(spec))
}

pub(super) fn is_absolute_local(spec: &str) -> bool {
    let unc = spec.starts_with("\\\\") || spec.starts_with("//");
    unc || std::path::Path::new(spec).is_absolute()
}

/// `a/./b/../c` → `a/c`; `\` separates like `/`; "" and "." are the root.
pub(super) fn normalize_relative(spec: &str, root: &str) -> Result<String, String> {
    if spec.contains('\0') {
        return Err("a path cannot contain a NUL byte".to_string());
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in spec.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!(
                        "{spec:?} leaves the working root {root}; `..` cannot go above it (give an absolute target or set another root)"
                    ));
                }
            }
            name => parts.push(name),
        }
    }
    Ok(parts.join("/"))
}

/// The rest of `spec` after the root `root` ("" for the root itself), or
/// `None` when `spec` is not the root or below it.
fn below<'a>(spec: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim();
    let trimmed = root.trim_end_matches(['/', '\\']);
    // A root of only separators (`/`) or a scheme root (`gdrive:///`) keeps
    // its last separator.
    let base = if trimmed.is_empty() || trimmed.ends_with(':') {
        root
    } else {
        trimmed
    };
    if spec == base || spec == root {
        return Some("");
    }
    let rest = spec.strip_prefix(base)?;
    if base.ends_with(['/', '\\']) {
        return Some(rest);
    }
    rest.strip_prefix(['/', '\\'])
}

/// The absolute target of `rel` below the root `root`.
pub(super) fn join_spec(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else if root.ends_with(['/', '\\']) {
        format!("{root}{rel}")
    } else {
        format!("{root}/{rel}")
    }
}

/// The Share mount that holds `path` (`/Docs/a` → `/Docs`, an exported
/// connection `/Verbindungen/<name>/…` → `/Verbindungen/<name>`); `None` for
/// the peer's virtual folders above the mounts. Mirrors the rename scope of
/// `cli::target`: one mount is one connection and one rename scope.
pub(super) fn peer_scope(path: &str) -> Option<String> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    if first == "Verbindungen" {
        return parts.next().map(|second| format!("/{first}/{second}"));
    }
    Some(format!("/{first}"))
}

/// Whether `path` of a root of `kind` is the root of its namespace, which a
/// removal must not take without `allow_root` (as `se rm --preserve-root`).
pub(super) fn namespace_root(kind: RootKind, path: &str) -> bool {
    let path = path.replace('\\', "/");
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return true;
    }
    match kind {
        RootKind::Unc | RootKind::WindowsShare => trimmed
            .strip_prefix("//")
            .is_some_and(|rest| rest.split('/').filter(|part| !part.is_empty()).count() <= 2),
        _ => {
            let bytes = trimmed.as_bytes();
            bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
        }
    }
}

/// The kind of the absolute target `spec`, from its spelling and the saved
/// connections (no network).
pub(super) fn kind_of(spec: &str) -> Result<RootKind, String> {
    let normalized = spec.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if spec.starts_with('@') {
        let (connection, _) = crate::cli::target::saved_shorthand(spec)?
            .ok_or_else(|| format!("unknown saved connection shorthand: {spec}"))?;
        return Ok(saved_kind(connection.protocol));
    }
    if crate::share::PeerOpenTarget::from_endpoint(&normalized).is_some() {
        return Ok(RootKind::Peer);
    }
    if lower.starts_with("gdrive://") {
        return Ok(RootKind::Drive);
    }
    if lower.starts_with("vault://") || lower.starts_with("vault-store://") {
        return Ok(RootKind::Vault);
    }
    if lower.contains("://") {
        return Ok(crate::connect::saved_and_path(&normalized)
            .map_or(RootKind::Url, |(connection, _)| {
                saved_kind(connection.protocol)
            }));
    }
    if normalized.starts_with("//") {
        return Ok(RootKind::Unc);
    }
    Ok(RootKind::Local)
}

fn saved_kind(protocol: Protocol) -> RootKind {
    match protocol {
        Protocol::Smb => RootKind::Smb,
        Protocol::Share => RootKind::WindowsShare,
        login => RootKind::Login(login),
    }
}

#[cfg(test)]
#[path = "root_path_tests.rs"]
mod tests;
