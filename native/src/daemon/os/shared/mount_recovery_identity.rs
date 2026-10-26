//! Recheck historical cached-write authority on every fresh mount generation.
use crate::mount::{MountConfig, MountSource, RecoveryBackendIdentity};
use crate::vfs::BackendHandle;
use std::{io, path::Path};

pub(super) fn authorize(
    config: &MountConfig,
    backend: &BackendHandle,
    cache: &Path,
) -> Result<(), String> {
    if !matches!(config.source, MountSource::GoogleDrive { .. }) {
        return Ok(());
    }
    // The lease is non-blocking and precedes every read/write below. It
    // creates only empty cache directories and is dropped before host start.
    let _lease = crate::mount::RecoveryCacheLease::acquire(cache, &config.id)
        .map_err(|error| format!("Laufwerkidentität exklusiv prüfen: {error}"))?;
    let expected = crate::mount::preflight_mount_recovery_identity(cache, config)
        .map_err(|error| error.to_string())?;
    let current = capture(config, backend, expected.as_ref()).map_err(|error| error.to_string())?;
    crate::mount::bind_mount_recovery_identity(cache, config, &current, &_lease)
        .map_err(|error| error.to_string())
}

fn capture(
    config: &MountConfig,
    backend: &BackendHandle,
    expected: Option<&RecoveryBackendIdentity>,
) -> io::Result<RecoveryBackendIdentity> {
    let state = backend.state_identity();
    let namespace = backend.namespace_identity();
    if let Some(expected) = expected {
        expected.validate_for(&config.source)?;
        expected.matches_session(&state, &namespace)?;
    }
    let principal = namespace
        .strip_prefix("gdrive:path-v2:")
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(crate::mount::recovery_identity_conflict)?;
    if backend.scheme() != crate::vfs::Scheme::GDrive {
        return Err(crate::mount::recovery_identity_conflict());
    }
    // This fresh backend validates the actual root ID and its folder bindings;
    // cached path hints and the provider's `root` alias are not the authority.
    let root_object = backend
        .item_id(config.source.root().as_str())?
        .filter(|id| !id.is_empty() && id != "root")
        .ok_or_else(crate::mount::recovery_identity_conflict)?;
    let current = RecoveryBackendIdentity {
        version: 1,
        source: config.source.clone(),
        provider: "gdrive".into(),
        principal: principal.into(),
        namespace,
        state_identity: state,
        root_object,
    };
    current.validate_for(&config.source)?;
    if expected.is_some_and(|expected| expected != &current) {
        return Err(crate::mount::recovery_identity_conflict());
    }
    Ok(current)
}
