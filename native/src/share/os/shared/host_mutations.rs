//! Host write adapters; authorization is the dispatcher's resolved-target contract.
use crate::{
    share::{
        fs::ResolvedTarget,
        wire::{FsRecycle, FsResponse, FsStageDurability, FsStageFinish, FsSyncFilesystem},
    },
    vfs::{RecycleExpectation, RecycleOutcome, StageDurability, StageFinish},
};
use std::io;

pub(in crate::share) fn recycle(
    target: ResolvedTarget,
    request: FsRecycle,
) -> io::Result<FsResponse> {
    if request.expected_sha256.as_ref().is_some_and(|hash| {
        hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ungültige erwartete SHA-256",
        ));
    }
    let expected = RecycleExpectation {
        size: request.expected_size,
        sha256: request.expected_sha256,
    };
    let result = if target.backend.scheme() == crate::vfs::Scheme::Local {
        let root = target.backend.root_display();
        crate::analytics::recycle_local(
            std::path::Path::new(&root),
            std::path::Path::new(&target.path),
            &expected,
        )
        .map_err(|error| {
            eprintln!("Host recycle {}: {error}", target.path);
            let physical =
                std::fs::canonicalize(&root).unwrap_or_else(|_| std::path::PathBuf::from(&root));
            let physical = crate::local_access::display_path(&physical);
            let relative = target
                .path
                .replace('\\', "/")
                .strip_prefix(physical.trim_end_matches('/'))
                .unwrap_or("")
                .trim_start_matches('/')
                .to_owned();
            let visible = request
                .path
                .strip_suffix(relative.as_str())
                .unwrap_or(&request.path)
                .trim_end_matches('/');
            io::Error::new(
                error.kind(),
                error
                    .to_string()
                    .replace(&physical, visible)
                    .replace(&root, visible),
            )
        })?
    } else {
        crate::vfs::recycle(&*target.backend, &target.path, &expected)?
    };
    Ok(FsResponse::Recycle {
        moved: result == RecycleOutcome::Recycled,
    })
}
pub(in crate::share) fn finish(
    target: ResolvedTarget,
    request: FsStageFinish,
) -> io::Result<FsResponse> {
    let name = target.path.rsplit(['/', '\\']).next().unwrap_or_default();
    if !crate::vfs::is_staging_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Nur eine eigene unveröffentlichte Stage darf abgeschlossen werden",
        ));
    }
    let durability = match request.durability {
        FsStageDurability::NotRequired => StageDurability::NotRequired,
        FsStageDurability::Deferred => StageDurability::Deferred,
        FsStageDurability::Now => StageDurability::Now,
        FsStageDurability::Unknown => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unbekannte Haltbarkeitsanforderung",
            ))
        }
    };
    let result = crate::vfs::finish_stage(
        &*target.backend,
        &target.path,
        StageFinish {
            mtime_ms: request.mtime_ms,
            mode: request.mode.map(|mode| mode & 0o777),
            durability,
        },
    )?;
    Ok(FsResponse::StageFinished {
        mtime_applied: result.mtime_applied,
        durable: result.durable,
    })
}
pub(in crate::share) fn sync(
    target: ResolvedTarget,
    _: FsSyncFilesystem,
) -> io::Result<FsResponse> {
    Ok(FsResponse::Synced {
        durable: crate::vfs::sync_filesystem(&*target.backend, &target.path)?,
    })
}
