//! Looks at the selected entries and the target while the roots are planned.
use super::super::walk_listers::native;
use super::view::Side;

pub(super) fn selected_directory(
    side: Side<'_>,
    path: &str,
    gate: Option<&crate::transfer::AccessGate>,
) -> Result<bool, String> {
    let inspect = || match side {
        Side::Local => crate::local_access::symlink_metadata(&native(path)).map(|meta| {
            meta.is_dir() && !crate::local_access::metadata_is_link_like(&native(path), &meta)
        }),
        Side::Remote(backend) => backend
            .stat(path)
            .map(|meta| meta.is_dir && !meta.is_symlink),
    };
    let mut result = inspect();
    if result
        .as_ref()
        .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
    {
        if let Some(gate) = gate {
            if gate.request() == crate::transfer::AccessAnswer::Granted {
                result = inspect();
            }
        }
    }
    match result {
        // A vanished entry is no folder to reserve: the walk reports it as
        // that root's own error and the other roots still go.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        result => result.map_err(|error| format!("Quellordner „{path}“ prüfen: {error}")),
    }
}

pub(super) fn target_exists(side: Side<'_>, path: &str) -> Result<bool, String> {
    let result = match side {
        Side::Local => crate::local_access::symlink_metadata(&native(path)).map(|_| ()),
        Side::Remote(backend) => backend.stat(path).map(|_| ()),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Zielordner „{path}“ prüfen: {error}")),
    }
}
