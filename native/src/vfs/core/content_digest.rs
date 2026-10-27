//! The SHA-256 of one regular file, computed on the device that stores it
//! (`content_digest`). Local backends read their own disk; remote backends
//! with a runtime next to the data (Share hosts and room members, the SSH
//! agent) hash there and send only the digest. Every other backend answers
//! `Ok(None)`: this layer never moves file content over the network to
//! obtain a digest, the caller decides whether a read-back is warranted.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Backend, VfsMeta, VfsResult};

/// One sequential read while hashing; cancel is checked between reads.
const HASH_CHUNK: usize = 1024 * 1024;

/// Digest of a file's whole content at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentDigest {
    /// 64 lowercase hex digits.
    pub sha256: String,
    pub size: u64,
}

/// SHA-256 of the regular file `path`, computed where the file is stored.
/// `Ok(None)`: this backend cannot do that without moving the content over
/// the network (or would reveal what it must not, such as a locked vault).
/// A file that is no regular file, changes or vanishes while it is hashed is
/// an error, never a digest; a cancel is `ErrorKind::Interrupted`.
pub fn content_digest(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
) -> VfsResult<Option<ContentDigest>> {
    check_cancel(cancel)?;
    let Some(extensions) = backend.extensions() else {
        return Ok(None);
    };
    let digest = extensions.content_digest(path, cancel)?;
    if let Some(found) = &digest {
        if !is_sha256_hex(&found.sha256) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "content digest is no lowercase hex SHA-256",
            ));
        }
    }
    Ok(digest)
}

/// Digest of a backend that stores the file on this device: stat, read the
/// regular file sequentially, stat again. Size and time must not change and
/// the read must end exactly at the stated size.
pub(crate) fn digest_by_local_read<B: Backend + ?Sized>(
    backend: &B,
    path: &str,
    cancel: &AtomicBool,
) -> VfsResult<Option<ContentDigest>> {
    check_cancel(cancel)?;
    let before = backend.stat(path)?;
    ensure_regular(&before, path)?;
    let sha256 = {
        let mut reader =
            super::extension_calls::open_read_regular(backend, path, before.id.as_deref())?;
        sha256_exact(&mut *reader, before.size, cancel, &mut |_| {})?
    };
    let after = backend.stat(path)?;
    ensure_regular(&after, path)?;
    if after.size != before.size || after.mtime_ms != before.mtime_ms {
        return Err(changed());
    }
    Ok(Some(ContentDigest {
        sha256,
        size: before.size,
    }))
}

/// SHA-256 (lowercase hex) of exactly `expected` bytes from `source`; more or
/// fewer bytes mean the file changed and are an error. `progress` receives
/// every hashed portion's length.
pub(crate) fn sha256_exact(
    source: &mut dyn Read,
    expected: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<String> {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0u8; HASH_CHUNK];
    let mut total = 0u64;
    loop {
        check_cancel(cancel)?;
        let count = match source.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > expected {
            return Err(changed());
        }
        context.update(&buffer[..count]);
        progress(count as u64);
    }
    if total != expected {
        return Err(changed());
    }
    Ok(lower_hex(context.finish().as_ref()))
}

/// 64 lowercase hex digits.
pub(crate) fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(is_lower_hex_digit)
}

fn is_lower_hex_digit(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn ensure_regular(meta: &VfsMeta, path: &str) -> io::Result<()> {
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no regular file: {path}"),
        ));
    }
    Ok(())
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "file changed while its content digest was computed",
    )
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "content digest canceled",
        ));
    }
    Ok(())
}
