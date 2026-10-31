use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::types::CompareMode;
pub(crate) use crate::agent_proto::LocalReads;

/// First 8 bytes of a 16-byte MD5 digest folded into a u64. Zero remains the
/// sentinel for "not hashed", so a real zero prefix is bumped to one.
pub(super) fn md5_to_u64(digest: &[u8; 16]) -> u64 {
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(prefix).max(1)
}

pub(crate) fn md5_hex_to_u64(hex: &str) -> u64 {
    let hex = hex.trim();
    if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return 0;
    }
    let mut digest = [0u8; 16];
    for index in 0..16 {
        let Ok(byte) = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16) else {
            return 0;
        };
        digest[index] = byte;
    }
    md5_to_u64(&digest)
}

pub(crate) fn hash_file(backend: &dyn Backend, path: &str, cancel: &AtomicBool) -> io::Result<u64> {
    md5_hex_file(backend, path, cancel).map(|hex| md5_hex_to_u64(&hex))
}

/// `hash_file` of a local side through the digest cache of this device
/// (`agent_proto::hash_cache`, E9): a file whose status is unchanged since
/// its digest was taken (and not racy) is not read again. With `reads.fresh`
/// (the checksum compare mode) the file is always read, and the digest read
/// is stored. With `reads.run` the file is read at most once per run, also
/// when the pair's key phase reads the same tree on another thread.
pub(crate) fn hash_file_cached(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
    reads: LocalReads<'_>,
) -> io::Result<u64> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if backend.is_local() && metadata.is_file() => metadata,
        _ => return hash_file(backend, path, cancel),
    };
    crate::agent_proto::hash_cache::configure_once(|| {
        (!cfg!(test)).then(crate::support_dirs::app_data_dir)
    });
    crate::agent_proto::md5_cached(std::path::Path::new(path), &metadata, reads, cancel, || {
        md5_hex_file(backend, path, cancel)
    })
    .map(|hex| md5_hex_to_u64(&hex))
}

/// The MD5 of a file as 32 lowercase hex digits, read through `backend`.
fn md5_hex_file(backend: &dyn Backend, path: &str, cancel: &AtomicBool) -> io::Result<String> {
    use std::io::Read;

    let captured = super::apply_guard::capture(
        backend,
        path,
        super::apply_guard::ExpectedFile::Unknown,
        "checksum file",
    )?;
    let metadata = captured.regular("checksum file")?;
    let mut reader = crate::vfs::open_read_regular(backend, path, metadata.id.as_deref())?;
    let mut context = md5::Context::new();
    let mut buffer = [0u8; 65_536];
    let mut length = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "checksum walk canceled",
            ));
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                length = length.saturating_add(read as u64);
                context.consume(&buffer[..read]);
            }
            Err(error) => return Err(error),
        }
    }
    if length != metadata.size {
        return Err(super::apply_guard::drift("checksum stream length changed"));
    }
    super::apply_guard::revalidate(backend, path, &captured, "checksum file")?;
    Ok(format!("{:x}", context.compute()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashMode {
    None,
    NativeOnly,
    /// Hash when needed, but reuse a prior nonzero hash for unchanged metadata.
    Full,
    /// Checksum mode: require a current nonzero hash and never reuse a previous
    /// size+mtime result as proof of unchanged content. A local side reads
    /// every file in this mode (`LocalReads::fresh`: the digest cache of the
    /// device is not asked, and takes the digest read); only a digest read
    /// earlier in the same run counts (`RunDigests`). A remote host's walk
    /// answers from its own cache while a file's status is unchanged:
    /// device, inode, size, modification and change time (ns), on file
    /// systems with durable file IDs, and not racy (`se doctor
    /// --clear-hash-cache` forgets the digests of this device).
    FullFresh,
}

pub(super) fn hash_mode(this: &dyn Backend, other: &dyn Backend, compare: CompareMode) -> HashMode {
    match compare {
        CompareMode::SizeOnly => HashMode::None,
        CompareMode::Checksum => HashMode::FullFresh,
        CompareMode::MtimeSize if this.provides_content_hash() => HashMode::NativeOnly,
        CompareMode::MtimeSize if this.is_local() && other.provides_content_hash() => {
            HashMode::Full
        }
        CompareMode::MtimeSize => HashMode::None,
    }
}

#[cfg(test)]
#[path = "snapshot_hash_task_tests.rs"]
mod task_tests;
