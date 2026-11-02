//! Stable file object identity, explicitly supplied by the guarding backend.
use super::{Backend, VfsResult};

/// `None` means unsupported or unknown, never evidence of ownership. A
/// backend must opt in through its own extension so wrapper root/endpoint
/// authority cannot be bypassed by a generic metadata-ID fallback.
pub fn file_identity<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<Option<String>> {
    let result = match backend.extensions() {
        Some(extensions) => extensions.file_identity(path),
        None => Ok(None),
    };
    match result {
        Ok(identity) => Ok(identity.filter(|identity| !identity.is_empty())),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => Ok(None),
        Err(error) => Err(error),
    }
}
