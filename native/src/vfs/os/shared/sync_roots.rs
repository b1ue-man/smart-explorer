//! Refuse overlap within one filesystem, while allowing `/Docs` on two remotes.
use std::io;

use super::{Backend, BackendHandle};

pub(crate) fn sync_backend(backend: BackendHandle) -> BackendHandle {
    backend.uncached_backend().unwrap_or(backend)
}

pub(crate) fn validate_sync_roots(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
) -> io::Result<()> {
    if root_a.is_empty() || root_b.is_empty() {
        return Err(invalid("Quelle und Ziel dürfen nicht leer sein."));
    }
    if a.is_local() && b.is_local() {
        if let (Ok(a), Ok(b)) = (
            std::fs::canonicalize(super::local_platform::to_os(root_a)),
            std::fs::canonicalize(super::local_platform::to_os(root_b)),
        ) {
            if a == b || a.starts_with(&b) || b.starts_with(&a) {
                return Err(overlap());
            }
        }
        crate::connect::validate_sync_endpoints(root_a, root_b).map_err(invalid)?;
    } else if a.namespace_identity() == b.namespace_identity()
        && crate::connect::location_paths_overlap(root_a, root_b)
    {
        return Err(overlap());
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn overlap() -> io::Error {
    invalid("Quelle und Ziel verweisen auf denselben oder verschachtelte Ordner.")
}
