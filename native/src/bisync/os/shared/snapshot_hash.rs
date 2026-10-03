use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::types::CompareMode;

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
    use std::io::Read;

    let captured = super::apply_guard::capture(backend, path,
        super::apply_guard::ExpectedFile::Unknown, "checksum file")?;
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
            Ok(read) => { length = length.saturating_add(read as u64); context.consume(&buffer[..read]); },
            Err(error) => return Err(error),
        }
    }
    if length != metadata.size { return Err(super::apply_guard::drift("checksum stream length changed")); }
    super::apply_guard::revalidate(backend, path, &captured, "checksum file")?;
    Ok(md5_to_u64(&context.compute().0))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashMode {
    None,
    NativeOnly,
    /// Hash when needed, but reuse a prior nonzero hash for unchanged metadata.
    Full,
    /// Checksum mode: require a current nonzero hash and never reuse a previous
    /// size+mtime result as proof of unchanged content.
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
