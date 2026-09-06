use std::{io::Write, path::Path};

use crate::mount::{MountId, MountRuntimePreference};
use super::{
    private_payload::{BUNDLED_DOKANY_BYTES, BUNDLED_DOKANY_SHA256, BUNDLED_DOKANY_SOURCE},
    runtime_attempt::RuntimeAttempt, DokanyPreflightError, DokanyRuntime,
};

pub(super) struct RuntimeSelection {
    pub(super) runtime: DokanyRuntime,
    attempt: Option<RuntimeAttempt>,
}

impl RuntimeSelection {
    /// A private rejection never weakens Windows policy or changes System32.
    /// CacheLease must already own this mount's cache and attempt namespace.
    pub(super) fn select(
        cache_root: &Path,
        id: &MountId,
        preference: MountRuntimePreference,
    ) -> Result<Self, DokanyPreflightError> {
        if preference == MountRuntimePreference::Auto && !BUNDLED_DOKANY_BYTES.is_empty() {
            match RuntimeAttempt::arm(cache_root, id) {
                Ok(attempt) => match DokanyRuntime::preflight_private(cache_root) {
                    Ok(runtime) => {
                        let _ = writeln!(std::io::stderr().lock(),
                            "mount runtime: private source={BUNDLED_DOKANY_SOURCE} sha256={BUNDLED_DOKANY_SHA256}");
                        return Ok(Self { runtime, attempt: Some(attempt) });
                    }
                    Err(error) => {
                        // No filesystem exists, so immediate fallback is safe.
                        // Keep the marker to avoid retrying this rejected input.
                        let _ = writeln!(std::io::stderr().lock(),
                            "mount runtime: private rejected ({error}); using official non-batched fallback");
                    }
                },
                Err(error) => {
                    let _ = writeln!(std::io::stderr().lock(),
                        "mount runtime: private attempt unavailable or unfinished ({error}); using official non-batched fallback");
                }
            }
        }
        Ok(Self { runtime: DokanyRuntime::preflight()?, attempt: None })
    }

    /// Call only after successful controlled teardown and recovery inspection.
    /// Any other exit leaves the marker for compatibility mode on the next Retry.
    pub(super) fn complete(self) {
        if let Some(attempt) = self.attempt {
            if let Err(error) = attempt.complete() {
                let _ = writeln!(std::io::stderr().lock(),
                    "mount runtime: retaining compatibility fallback marker: {error}");
            }
        }
    }
}
