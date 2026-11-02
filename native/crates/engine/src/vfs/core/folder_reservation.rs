//! Strict selected-folder reservation; never the legacy probe/create fallback.
use super::{Backend, VfsResult};

pub fn directory_reservation_supported(backend: &dyn Backend, root: &str) -> bool {
    backend
        .extensions()
        .is_some_and(|extensions| extensions.directory_reservation_supported(root))
}

pub fn create_dir_exclusive(backend: &dyn Backend, path: &str) -> VfsResult<()> {
    match backend.extensions() {
        Some(extensions) => extensions.create_dir_exclusive(path),
        None => super::meta::unsupported("backend cannot reserve a directory atomically"),
    }
}

/// Stable object identity, independent of its current name. `None` is an
/// unknown identity and cannot establish ownership after a lost response.
pub fn directory_identity(backend: &dyn Backend, path: &str) -> VfsResult<Option<String>> {
    match backend.extensions() {
        Some(extensions) => extensions
            .directory_identity(path)
            .map(|identity| identity.filter(|value| !value.is_empty())),
        None => Ok(None),
    }
}

/// The strict publication boundary for an owned reservation. Ordinary UI
/// folder renames keep their protocol's existing no-overwrite behavior.
pub fn rename_reserved_directory(
    backend: &dyn Backend,
    source: &str,
    destination: &str,
) -> VfsResult<()> {
    match backend.extensions() {
        Some(extensions) => extensions.rename_reserved_directory(source, destination),
        None => {
            super::meta::unsupported("backend cannot publish a directory reservation atomically")
        }
    }
}
