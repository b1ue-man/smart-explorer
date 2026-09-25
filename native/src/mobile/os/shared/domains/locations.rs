//! Location strings for paths found below a resolved location (analysis and
//! duplicate results), in the desktop endpoint format.
use crate::connect::RemovedEndpointScope;
use crate::mobile::Runtime;
use crate::share::PeerOpenTarget;

/// A local path (no `scheme://` prefix).
pub(super) fn is_local(location: &str) -> bool {
    !location.contains("://")
}

/// The location of backend `path` on the same endpoint as `base`.
pub(super) fn location_for(base: &str, path: &str) -> String {
    if is_local(base) {
        return path.to_string();
    }
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    if base.starts_with("gdrive://") {
        return crate::connect::gdrive_endpoint(&path);
    }
    if let Some((target, _)) = PeerOpenTarget::from_endpoint(base) {
        return format!("{}{}", target.endpoint_prefix(), path);
    }
    // `proto://user@host:port` is kept verbatim, so the saved connection
    // matches exactly as before.
    match base.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{authority}{path}")
        }
        None => path,
    }
}

/// `root` joined with child segments (`/` separated).
pub(super) fn join_segments(root: &str, segments: &[String]) -> String {
    let mut out = root.trim_end_matches('/').to_string();
    for segment in segments {
        out.push('/');
        out.push_str(segment);
    }
    if out.is_empty() {
        "/".to_string()
    } else {
        out
    }
}

/// After a connection, device or room was removed: its pooled sessions are
/// closed and its „Zuletzt“ entries dropped (favourites, folder preferences
/// and mounts are cleaned by `connect::cleanup_removed_endpoint_state`).
pub(super) fn forget_endpoint(rt: &Runtime, scope: &RemovedEndpointScope) {
    rt.drop_backends(&|key| scope.matches_key(key));
    rt.forget_recent(&|location| scope.matches_key(location));
}
