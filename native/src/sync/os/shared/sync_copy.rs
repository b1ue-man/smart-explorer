//! Mirror transfers share exclusive stages, version backups and durability with bisync.
use crate::bisync::{versions::RunVersions, Sig};
use crate::vfs::{Backend, VfsMeta};
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) struct CopyError {
    pub(super) error: io::Error,
    pub(super) publishing: bool,
}

pub(super) fn signature(meta: &VfsMeta) -> Sig {
    Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: meta
            .content_md5
            .as_deref()
            .map_or(0, crate::bisync::transfer_stream::native_hash),
    }
}

pub(super) fn unchanged(expected: &VfsMeta, actual: &VfsMeta) -> bool {
    !actual.is_dir
        && !actual.is_symlink
        && !actual.special
        && actual.size == expected.size
        && actual.mtime_ms == expected.mtime_ms
        && actual.id == expected.id
        && !matches!((&actual.content_md5, &expected.content_md5), (Some(a), Some(b)) if a != b)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_stream_scoped(
    source: &dyn Backend,
    source_root: &str,
    source_path: &str,
    source_rel: &str,
    source_expected: &VfsMeta,
    destination: &dyn Backend,
    destination_root: &str,
    destination_path: &str,
    destination_rel: &str,
    destination_expected: Option<&VfsMeta>,
    versions: &RunVersions,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u64, CopyError> {
    let preflight = (|| {
        crate::bisync::apply_boundary::guard(source, source_root, source_rel, false)?;
        crate::bisync::apply_boundary::guard(
            destination,
            destination_root,
            destination_rel,
            false,
        )?;
        crate::bisync::apply_boundary::target(
            destination,
            destination_root,
            destination_rel,
            Some(source_expected.size),
        )?;
        let actual = source.stat(source_path)?;
        if !unchanged(source_expected, &actual) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source changed before mirror transfer",
            ));
        }
        if let Some(expected) = destination_expected {
            if !unchanged(expected, &destination.stat(destination_path)?) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "destination changed before mirror transfer",
                ));
            }
        }
        Ok(())
    })();
    preflight.map_err(|error| CopyError {
        error,
        publishing: false,
    })?;
    crate::bisync::apply_transaction::quick_copy(
        source,
        source_path,
        signature(source_expected),
        destination,
        destination_path,
        destination_expected.map(signature),
        destination_root,
        destination_rel,
        versions,
        cancel,
        progress,
    )
    .map(|outcome| outcome.bytes)
    .map_err(|(error, publishing)| CopyError { error, publishing })
}

/// The previous internal signature remains available to older task fixtures.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_stream(
    source: &dyn Backend,
    source_path: &str,
    source_expected: &VfsMeta,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: Option<&VfsMeta>,
    _guard_parent: bool,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u64, CopyError> {
    let (source_root, source_rel) = source_path.rsplit_once('/').unwrap_or((source_path, ""));
    let (destination_root, destination_rel) = destination_path
        .rsplit_once('/')
        .unwrap_or((destination_path, ""));
    let run = super::sync_run::MirrorRun::begin(source, source_root, destination, destination_root)
        .map_err(|error| CopyError {
            error,
            publishing: false,
        })?;
    let result = copy_stream_scoped(
        source,
        source_root,
        source_path,
        source_rel,
        source_expected,
        destination,
        destination_root,
        destination_path,
        destination_rel,
        destination_expected,
        &run.versions,
        cancel,
        progress,
    );
    run.finish(destination, destination_root, cancel)
        .map_err(|error| CopyError {
            error,
            publishing: true,
        })?;
    result
}
