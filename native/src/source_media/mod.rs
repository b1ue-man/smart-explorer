//! Mounted optical media: physical-device arbitration, never a raw-disc reader.
mod permit;
#[cfg(target_os = "linux")]
#[path = "os/linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "os/unsupported.rs"]
mod platform;
mod reader;
mod status;

pub use permit::{
    acquire, acquire_current, acquire_file, acquire_file_current, acquire_source,
    with_cancellation, Guard, Priority,
};
pub use reader::{guard_file_reader, guard_reader, GuardedIterator};
pub use status::{snapshots, ReadMode, Status};
use std::{io, path::Path};

/// Device identity excludes mount aliases and media identity. Different discs in
/// the same drive must still contend on the same device lock.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Source {
    pub device: String,
    pub generation: String,
    pub read_only: bool,
}

/// OS evidence only. UDF/ISO filesystem names never classify a physical drive.
/// This may contact the mounted filesystem and belongs on a worker thread.
pub fn probe(path: &Path) -> io::Result<Option<Source>> {
    #[cfg(test)]
    if let Some(source) = test_profiles::probe(path) {
        return Ok(Some(source));
    }
    platform::probe(path)
}

/// DVD ECC unit is 32 KiB. Keep the selected block visible in progress and
/// measurable; no claim about a particular physical drive's throughput.
pub const OPTICAL_BLOCK_BYTES: usize = 32 * 32 * 1024;

pub fn sequential_copy(
    reader: &mut dyn io::Read,
    writer: &mut dyn io::Write,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    let mut buffer = vec![0; OPTICAL_BLOCK_BYTES];
    let mut total = 0u64;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) || permit::context_canceled() {
            return Ok(None);
        }
        let count = loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) || permit::context_canceled() {
                return Ok(None);
            }
            match reader.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        if count == 0 {
            return Ok(Some(total));
        }
        writer.write_all(&buffer[..count])?;
        total = total.saturating_add(count as u64);
        progress(count as u64);
    }
}

/// Marks the implementation selected while the caller owns its device guard.
pub fn mark_mode(path: &Path, mode: ReadMode) {
    if let Ok(Some(source)) = probe(path) {
        status::update(&source, |status| {
            if status.active {
                status.mode = mode;
                status.block_bytes = (mode == ReadMode::Sequential).then_some(OPTICAL_BLOCK_BYTES);
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod test_profiles;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;

/// Facts bound to an existing handle, preserving retained-root authority.
pub fn probe_file(file: &std::fs::File) -> io::Result<Option<Source>> {
    platform::probe_file(file)
}
