use crate::vfs::Backend;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply_guard::ExpectedFile;
use super::types::{CompareMode, Sig};
pub use crate::agent_proto::LocalReads;

/// First 8 bytes of a 16-byte MD5 digest folded into a u64. Zero remains the
/// sentinel for "not hashed", so a real zero prefix is bumped to one.
pub(super) fn md5_to_u64(digest: &[u8; 16]) -> u64 {
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(prefix).max(1)
}

pub fn md5_hex_to_u64(hex: &str) -> u64 {
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

pub fn hash_file(backend: &dyn Backend, path: &str, cancel: &AtomicBool) -> io::Result<u64> {
    md5_hex_file(backend, path, cancel).map(|hex| md5_hex_to_u64(&hex))
}

/// `hash_file` of a local side through the digest cache of this device
/// (`agent_proto::hash_cache`, E9): a file whose status is unchanged since
/// its digest was taken (and not racy) is not read again. With `reads.fresh`
/// (the checksum compare mode) the file is always read, and the digest read
/// is stored. With `reads.run` the file is read at most once per run
/// (`RunDigests`).
pub fn hash_file_cached(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
    reads: LocalReads<'_>,
) -> io::Result<u64> {
    hash_file_cached_hex(backend, path, cancel, reads).map(|hex| md5_hex_to_u64(&hex))
}

/// `hash_file_cached` as 32 lowercase hex digits (the digest a folder key
/// of the side covers, `snapshot_local_keys.rs`).
pub(super) fn hash_file_cached_hex(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
    reads: LocalReads<'_>,
) -> io::Result<String> {
    cached_hex(backend, path, ExpectedFile::Unknown, cancel, reads)
}

/// `hash_file_cached` of a file the planner saw as `planned`: the read checks
/// that size and time are still the planned ones when it starts and that the
/// identity is unchanged after it (`apply_guard`), so the caller needs no
/// status calls of its own around it (each one is a handle open on Windows).
pub(super) fn hash_planned(
    backend: &dyn Backend,
    path: &str,
    planned: Sig,
    cancel: &AtomicBool,
    reads: LocalReads<'_>,
) -> io::Result<u64> {
    let expected = ExpectedFile::Present(Sig { hash: 0, ..planned });
    cached_hex(backend, path, expected, cancel, reads).map(|hex| md5_hex_to_u64(&hex))
}

fn cached_hex(
    backend: &dyn Backend,
    path: &str,
    expected: ExpectedFile,
    cancel: &AtomicBool,
    reads: LocalReads<'_>,
) -> io::Result<String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if backend.is_local() && metadata.is_file() => metadata,
        _ => return md5_hex_expect(backend, path, expected, cancel),
    };
    crate::agent_proto::hash_cache::configure_once(|| {
        (!cfg!(test)).then(crate::support_dirs::app_data_dir)
    });
    crate::agent_proto::md5_cached(std::path::Path::new(path), &metadata, reads, cancel, || {
        md5_hex_expect(backend, path, expected, cancel)
    })
}

/// The MD5 of a file as 32 lowercase hex digits. A remote side that forms
/// the MD5 where the file is stored (`vfs::content_md5`: FTP `HASH`/`XMD5`,
/// SFTP by exec, WebDAV checksums, an agent next to the data) is asked
/// first and moves no content; only a side without one is read through
/// `backend`. Every caller of a remote side (incremental checksum
/// observation, index commit, move finalization, own-write recognition)
/// goes through here.
fn md5_hex_file(backend: &dyn Backend, path: &str, cancel: &AtomicBool) -> io::Result<String> {
    md5_hex_expect(backend, path, ExpectedFile::Unknown, cancel)
}

/// `md5_hex_file` of a file that must match `expected` when it is opened.
fn md5_hex_expect(
    backend: &dyn Backend,
    path: &str,
    expected: ExpectedFile,
    cancel: &AtomicBool,
) -> io::Result<String> {
    let captured = super::apply_guard::capture(backend, path, expected, "checksum file")?;
    let metadata = captured.regular("checksum file")?;
    if !backend.is_local() {
        if let Some(hex) = crate::vfs::content_md5(backend, path, cancel)? {
            super::apply_guard::revalidate(backend, path, &captured, "checksum file")?;
            return Ok(hex);
        }
    }
    let reader = crate::vfs::open_read_regular(backend, path, metadata.id.as_deref())?;
    let (digest, length) = if metadata.size > CHUNK as u64 {
        md5_read_ahead(reader, cancel)?
    } else {
        md5_inline(reader, cancel)?
    };
    if length != metadata.size {
        return Err(super::apply_guard::drift("checksum stream length changed"));
    }
    super::apply_guard::revalidate(backend, path, &captured, "checksum file")?;
    Ok(format!("{digest:x}"))
}

/// Read size of one chunk of a file larger than one chunk: such a file is
/// read one chunk ahead on a second thread while the MD5 consumes the chunk
/// before, so the disk (or the link) and the processor work at the same time
/// instead of taking turns. The MD5 itself runs in order and cannot be split.
const CHUNK: usize = 1 << 20;

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "checksum walk canceled")
}

fn md5_inline(
    mut reader: Box<dyn Read + Send>,
    cancel: &AtomicBool,
) -> io::Result<(md5::Digest, u64)> {
    let mut context = md5::Context::new();
    let mut buffer = [0u8; 65_536];
    let mut length = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
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
    Ok((context.compute(), length))
}

/// `md5_inline` with the next chunk read while this one is hashed. Two
/// buffers circulate; whichever side stops first (end, error, cancel) drops
/// its channel end, which ends the other.
fn md5_read_ahead(
    mut reader: Box<dyn Read + Send>,
    cancel: &AtomicBool,
) -> io::Result<(md5::Digest, u64)> {
    use std::sync::mpsc::sync_channel;
    let (full_tx, full_rx) = sync_channel::<io::Result<Vec<u8>>>(1);
    let (empty_tx, empty_rx) = sync_channel::<Vec<u8>>(2);
    for _ in 0..2 {
        let _ = empty_tx.send(vec![0u8; CHUNK]);
    }
    std::thread::scope(move |scope| {
        scope.spawn(move || {
            while let Ok(mut buffer) = empty_rx.recv() {
                if cancel.load(Ordering::Relaxed) {
                    let _ = full_tx.send(Err(canceled()));
                    return;
                }
                buffer.resize(CHUNK, 0);
                match fill(&mut *reader, &mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        buffer.truncate(read);
                        if full_tx.send(Ok(buffer)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = full_tx.send(Err(error));
                        return;
                    }
                }
            }
        });
        let mut context = md5::Context::new();
        let mut length = 0u64;
        for chunk in full_rx {
            let chunk = chunk?;
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            length = length.saturating_add(chunk.len() as u64);
            context.consume(&chunk);
            let _ = empty_tx.send(chunk);
        }
        Ok((context.compute(), length))
    })
}

/// Reads until `buffer` is full or the file ends; the count read.
fn fill(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
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
    /// device is not asked, and takes the digest read), once per run: its
    /// one walk forms its snapshot and its folder keys together. A walk next
    /// to the data reads every file as well: the request carries `fresh`
    /// (`snapshot_agent.rs`; agent and service `walk-filter-v1`, Share
    /// `hash_walk_filter_v1`), and the host then never answers from its own
    /// digest cache. Only a host without that feature answers from its cache
    /// while a file's status is unchanged: device, inode, size, modification
    /// and change time (ns), on file systems with durable file IDs, and not
    /// racy (`se doctor --clear-hash-cache` forgets the digests of a device).
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
