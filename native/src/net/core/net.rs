//! Authenticated network-share access via `WNetAddConnection2W` (mpr.dll).
//!
//! `\\server\share` UNC paths and mapped drive letters are *browsed* through
//! `LocalBackend` (std::fs) — no new filesystem code. This module only adds the
//! ability to authenticate to a share that needs credentials (deviceless: no
//! drive letter is mapped), after which the UNC path reads normally. The
//! connection is held open by `NetConnection` and torn down on drop.
//!
//! Local-network DISCOVERY (browsing the neighborhood) is intentionally NOT
//! here: it's unreliable on Win11 (SMB1 Computer Browser gone; WNetEnumResource
//! / NET VIEW widely broken). Connecting to a KNOWN address works; that's the
//! supported UX. See docs/GOTCHAS.md / REMOTE_LAYER_PLAN §4.
#![allow(dead_code)] // staged: wired in by the connect-UI step.

use std::io;

use super::platform::{connect_impl, disconnect_impl};

/// `\\server\share` (back-slash, canonical for WNet) from any path beneath it,
/// or `None` for a non-UNC path. Accepts forward- or back-slash input (the app
/// stores paths forward-slashed).
pub fn share_root(path: &str) -> Option<String> {
    if !is_unc(path) {
        return None;
    }
    let body = path
        .trim()
        .trim_start_matches(|c| c == '\\' || c == '/');
    let mut parts = body.split(|c| c == '\\' || c == '/').filter(|s| !s.is_empty());
    let server = parts.next()?;
    let share = parts.next()?;
    if server.is_empty() || share.is_empty() {
        return None;
    }
    Some(format!("\\\\{}\\{}", server, share))
}

/// Whether a path is a UNC path (`\\server\…` or `//server/…`).
pub fn is_unc(path: &str) -> bool {
    let p = path.trim_start();
    p.starts_with("\\\\") || p.starts_with("//")
}

/// A live authenticated connection to a network share. Dropping it releases the
/// connection (best-effort).
pub struct NetConnection {
    share: String, // \\server\share
}

impl NetConnection {
    /// Authenticate to the share that `unc` lives under. `user`/`password` may be
    /// `None` to use the caller's current credentials (Kerberos/NTLM SSO).
    pub fn connect(
        unc: &str,
        user: Option<&str>,
        password: Option<&str>,
    ) -> io::Result<NetConnection> {
        let share = share_root(unc).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "kein UNC-Pfad (\\\\server\\share)",
            )
        })?;
        connect_impl(&share, user, password)?;
        Ok(NetConnection { share })
    }

    pub fn share(&self) -> &str {
        &self.share
    }
}

impl Drop for NetConnection {
    fn drop(&mut self) {
        disconnect_impl(&self.share);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_detection() {
        assert!(is_unc(r"\\server\share"));
        assert!(is_unc("//server/share"));
        assert!(!is_unc(r"C:\Users"));
        assert!(!is_unc("/home/user"));
        assert!(!is_unc("sftp://h/p"));
    }

    #[test]
    fn share_root_extraction() {
        assert_eq!(share_root(r"\\srv\pub\a\b").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root("//srv/pub/a/b").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root(r"\\srv\pub").as_deref(), Some(r"\\srv\pub"));
        assert_eq!(share_root(r"\\srv").as_deref(), None); // no share component
        assert_eq!(share_root(r"C:\x").as_deref(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn connect_unsupported_off_windows() {
        let err = NetConnection::connect(r"\\srv\pub", Some("u"), Some("p"))
            .err()
            .expect("must error off-Windows");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn connect_rejects_non_unc() {
        let err = NetConnection::connect("C:/x", None, None)
            .err()
            .expect("non-UNC must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
