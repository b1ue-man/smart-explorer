//! Longer default bodies of `Backend` methods, kept out of the trait file.
use std::io;

use super::{Backend, VfsResult};

/// `Backend::try_exists`: only `NotFound` means absent.
pub(super) fn try_exists<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<bool> {
    match backend.stat(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// `Backend::create_dir_new`: probe first (not atomic), then create.
pub(super) fn create_dir_new<B: Backend + ?Sized>(backend: &B, path: &str) -> VfsResult<()> {
    if backend.try_exists(path)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            path.to_string(),
        ));
    }
    backend.create_dir(path)
}
