//! Completion of a move, including a copy already confirmed in an earlier run.
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::apply_retry::AttemptError;
use super::checkpoint::ApplyScope;
use super::transfer_stream::stream;
use super::types::{BisyncOptions, Sig};
use super::versions::VersionSide;
use crate::vfs::Backend;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub(super) fn checked_pair(
    source: &dyn Backend,
    source_path: &str,
    source_expected: ExpectedFile,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: ExpectedFile,
    cancel: &AtomicBool,
) -> io::Result<(CapturedFile, CapturedFile, Sig, Sig)> {
    let source_state = capture(source, source_path, source_expected, "move source")?;
    let destination_state = capture(
        destination,
        destination_path,
        destination_expected,
        "move destination",
    )?;
    let sm = source_state.regular("move source")?;
    let dm = destination_state.regular("move destination")?;
    // Read sessions sequentially: FTP and similar single-session providers
    // cannot keep two readers alive together.
    let source_bytes = {
        let mut reader = crate::vfs::open_read_regular(source, source_path, sm.id.as_deref())?;
        stream(
            &mut *reader,
            &mut io::sink(),
            cancel,
            None,
            source_expected.hash(),
            |_| {},
        )?
    };
    let destination_bytes = {
        let mut reader =
            crate::vfs::open_read_regular(destination, destination_path, dm.id.as_deref())?;
        stream(
            &mut *reader,
            &mut io::sink(),
            cancel,
            None,
            destination_expected.hash(),
            |_| {},
        )?
    };
    if (source.download_name(source_path, &sm.name) == sm.name && source_bytes.bytes != sm.size)
        || (destination.download_name(destination_path, &dm.name) == dm.name
            && destination_bytes.bytes != dm.size)
        || source_bytes.bytes != destination_bytes.bytes
        || source_bytes.digest != destination_bytes.digest
    {
        return Err(drift(
            "move destination differs from source; source retained",
        ));
    }
    revalidate(source, source_path, &source_state, "move source")?;
    revalidate(
        destination,
        destination_path,
        &destination_state,
        "move destination",
    )?;
    let source_sig = Sig {
        size: sm.size,
        mtime_ms: sm.mtime_ms,
        hash: source_bytes.hash(),
    };
    let destination_sig = Sig {
        size: dm.size,
        mtime_ms: dm.mtime_ms,
        hash: destination_bytes.hash(),
    };
    Ok((source_state, destination_state, source_sig, destination_sig))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_scoped(
    source: &VersionSide<'_>,
    source_path: &str,
    source_sig: Sig,
    destination: &dyn Backend,
    destination_path: &str,
    destination_sig: Sig,
    rel: &str,
    opts: BisyncOptions,
    scope: &ApplyScope<'_>,
    cancel: &AtomicBool,
) -> Result<bool, AttemptError> {
    let destination_state = capture(
        destination,
        destination_path,
        ExpectedFile::Present(destination_sig),
        "move destination",
    )
    .map_err(AttemptError::pre_commit)?;
    super::apply_transaction::delete(
        source,
        source_path,
        ExpectedFile::Present(source_sig),
        rel,
        opts,
        Some(scope.versions),
        cancel,
        Some(scope.sink),
        || {
            let hash = super::snapshot_hash::hash_file(destination, destination_path, cancel)?;
            if hash != destination_sig.hash {
                return Err(drift("move destination content changed; source retained"));
            }
            revalidate(
                destination,
                destination_path,
                &destination_state,
                "move destination",
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify_and_delete_source(
    source: &dyn Backend,
    source_path: &str,
    destination: &dyn Backend,
    destination_path: &str,
    rel: &str,
    reversible: bool,
    versions_dir: &Path,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (_, destination_state, source_sig, destination_sig) = checked_pair(
        source,
        source_path,
        ExpectedFile::Unknown,
        destination,
        destination_path,
        ExpectedFile::Unknown,
        cancel,
    )?;
    super::apply_delete::delete_guarded_with_progress_and_guard(
        source,
        source_path,
        rel,
        ExpectedFile::Present(source_sig),
        reversible,
        versions_dir,
        false,
        cancel,
        |_| {},
        || {
            let hash = super::snapshot_hash::hash_file(destination, destination_path, cancel)?;
            if hash != destination_sig.hash {
                return Err(drift("move destination content changed; source retained"));
            }
            revalidate(
                destination,
                destination_path,
                &destination_state,
                "move destination",
            )
        },
    )
    .map_err(AttemptError::into_io)?;
    super::apply_stage::require_durable(super::apply_stage::namespace(source, source_path)?)
}
