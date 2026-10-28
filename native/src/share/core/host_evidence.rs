//! Host side of `evidence_v1`: what a Stat and a FileIdentity request of the
//! same path would answer, read through the same access boundary while the
//! host serves or publishes the file anyway. The export scope an identity is
//! bound to is computed once per export and request (`Scopes`), not once
//! per file.
use std::collections::HashMap;
use std::io;

use super::fs::ResolvedTarget;
use super::fs_access::FsAccess;
use super::peer_extensions::file_identity;
use super::wire::FsFileEvidence;
use crate::vfs::VfsMeta;

/// Identity scopes of one request, by export root and mount.
#[derive(Default)]
pub(super) struct Scopes(HashMap<(String, String), String>);

impl Scopes {
    fn scope(
        &mut self,
        access: &FsAccess,
        path: &str,
        target: &ResolvedTarget,
    ) -> io::Result<String> {
        let key = (export_root(path)?, target.mount_key.clone());
        if let Some(scope) = self.0.get(&key) {
            return Ok(scope.clone());
        }
        let scope = file_identity::scope_of(access, path, target)?;
        self.0.insert(key, scope.clone());
        Ok(scope)
    }
}

/// The export (or exported connection) a peer path lies in: the part of the
/// path `file_identity` binds its scope to.
fn export_root(path: &str) -> io::Result<String> {
    let parts = super::fs::split_clean(path)?;
    let depth = if parts.first().is_some_and(|name| name == "Verbindungen") {
        2
    } else {
        1
    };
    Ok(parts
        .iter()
        .take(depth)
        .cloned()
        .collect::<Vec<_>>()
        .join("/"))
}

/// Evidence of the regular file `target` (the peer path `path`) whose
/// metadata `meta` the caller just read; `None` for anything else or when
/// its identity cannot be read (the client then asks as before).
pub(super) fn of_meta(
    access: &FsAccess,
    path: &str,
    target: &ResolvedTarget,
    meta: &VfsMeta,
    scopes: &mut Scopes,
) -> Option<FsFileEvidence> {
    if meta.is_dir || meta.is_symlink || meta.special {
        return None;
    }
    let scope = scopes.scope(access, path, target).ok()?;
    let identity = file_identity::bound_with_scope(&scope, target).ok()?;
    Some(FsFileEvidence {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        id: meta.id.clone(),
        identity,
    })
}

/// `of_meta` after one stat of `target`.
pub(super) fn of_target(
    access: &FsAccess,
    path: &str,
    target: &ResolvedTarget,
    scopes: &mut Scopes,
) -> Option<FsFileEvidence> {
    let meta = target.backend.stat(&target.path).ok()?;
    of_meta(access, path, target, &meta, scopes)
}

/// Whether `target` exists (`None` when that cannot be told): a link or
/// special file counts as existing, it takes the name all the same.
pub(super) fn exists(target: &ResolvedTarget) -> Option<bool> {
    match target.backend.stat(&target.path) {
        Ok(_) => Some(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}
