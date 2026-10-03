//! Quick Mirror consumes the same guarded, reversible apply primitives.
use super::apply_guard::ExpectedFile;
use super::apply_retry::AttemptError;
use super::apply_stage::{require_durable, CopyOutcome};
use super::apply_transaction::{copy, delete};
use super::types::{BisyncOptions, Throttle};
use super::versions::{RunVersions, VersionSide};
use crate::vfs::{Backend, StageDurability};
use std::io;
use std::sync::atomic::AtomicBool;

/// Compatibility entry used by Quick Mirror; literal backend paths stay at
/// the shared boundary. The caller owns the pair lock and run versions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quick_copy(
    source: &dyn Backend,
    source_path: &str,
    expected_source: super::types::Sig,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: Option<super::types::Sig>,
    root: &str,
    rel: &str,
    versions: &RunVersions,
    cancel: &AtomicBool,
    bytes: impl FnMut(u64),
) -> Result<CopyOutcome, (io::Error, bool)> {
    let outcome = copy(
        source,
        source_path,
        ExpectedFile::Present(expected_source),
        &VersionSide {
            side: super::types::PairSide::B,
            backend: destination,
            root,
        },
        destination_path,
        destination_expected.map_or(ExpectedFile::Missing, ExpectedFile::Present),
        rel,
        BisyncOptions {
            versions: versions.context().location,
            ..Default::default()
        },
        Some(versions),
        StageDurability::Now,
        &Throttle::new(0),
        cancel,
        None,
        |_| {},
        bytes,
    )
    .map_err(|failure| {
        let publishing = !failure.before_commit();
        (failure.into_io(), publishing)
    })?;
    require_durable(outcome.durable).map_err(|error| (error, true))?;
    Ok(outcome)
}

/// Mirror orphans use the same reversible delete boundary as planned apply.
pub(crate) fn quick_delete(
    backend: &dyn Backend,
    root: &str,
    path: &str,
    rel: &str,
    expected: super::types::Sig,
    versions: &RunVersions,
    cancel: &AtomicBool,
    guard: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let durable = delete(
        &VersionSide {
            side: super::types::PairSide::B,
            backend,
            root,
        },
        path,
        ExpectedFile::Present(expected),
        rel,
        BisyncOptions {
            versions: versions.context().location,
            ..Default::default()
        },
        Some(versions),
        cancel,
        None,
        guard,
    )
    .map_err(AttemptError::into_io)?;
    require_durable(durable)
}
