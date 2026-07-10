use super::Backend;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

const UNIQUE_ATTEMPTS: u32 = 1000;

/// Allocate an absent, hard-to-guess sibling name. Probe failures are errors,
/// never interpreted as a free path.
pub fn unique_staging_path<B: Backend + ?Sized>(
    backend: &B,
    destination: &str,
    purpose: &str,
) -> io::Result<String> {
    for attempt in 0..UNIQUE_ATTEMPTS {
        let candidate = format!(
            "{destination}.se-{purpose}-{:016x}",
            random_suffix(destination, purpose, attempt)
        );
        if !backend.try_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("could not allocate a unique {purpose} sibling"),
    ))
}

/// Promote a fully flushed staged file through the backend's declared safe
/// commit primitive.
pub fn promote_staged_replace<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    backend.promote_staged(staged, destination)
}

/// Promote a fully flushed staged file only if the destination remains absent.
/// The backend contract forbids implementing this as a probe followed by a
/// replacing rename.
pub fn promote_staged_create<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    backend.promote_staged_no_replace(staged, destination)
}

pub(crate) fn default_promote_staged_no_replace<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    validate_staged_file(backend, staged)?;
    backend.rename_no_replace(staged, destination)
}

pub(crate) fn default_promote_staged<B: Backend + ?Sized>(
    backend: &B,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    validate_staged_file(backend, staged)?;

    if !backend.try_exists(destination)? {
        return backend.rename_no_replace(staged, destination);
    }

    let destination_meta = backend.stat(destination)?;
    if destination_meta.is_dir || destination_meta.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to replace a directory or link-like destination with a file",
        ));
    }
    if !backend.rename_overwrites() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "backend cannot atomically replace an existing file",
        ));
    }
    backend.rename(staged, destination)
}

fn validate_staged_file<B: Backend + ?Sized>(backend: &B, staged: &str) -> io::Result<()> {
    let staged_meta = backend.stat(staged)?;
    if staged_meta.is_dir || staged_meta.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staged promotion source must be a regular file",
        ));
    }
    Ok(())
}

fn random_suffix(destination: &str, purpose: &str, attempt: u32) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    destination.hash(&mut hasher);
    purpose.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    attempt.hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
}
