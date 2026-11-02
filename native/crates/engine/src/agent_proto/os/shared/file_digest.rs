//! The MD5 of one local file for the walks that run in this process (a local
//! sync side, `bisync::snapshot_hash`, `local_keyed`) and for the agent's own
//! walks: answered by the device's digest cache (`hash_cache`) while the
//! file's status is unchanged, or read; with `fresh` (the checksum compare
//! mode) always read, and the digest read is stored. With a run's digests
//! (`RunDigests`) the file is also read at most once per run, whichever walk
//! of the side asks first.
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::UNIX_EPOCH;

use super::ops_types::digest;
use super::run_digests::{FileKey, RunDigests};

/// How a walk in this process reads the contents of files.
#[derive(Clone, Copy, Default)]
pub struct LocalReads<'a> {
    /// Never take a digest from the cache (the checksum compare mode).
    pub fresh: bool,
    /// The digests of this run, shared with the other walk of the side.
    pub run: Option<&'a RunDigests>,
}

/// The file `path` (`metadata`: its own, a link not followed) as the run's
/// digests name it: the path without `.` parts and doubled separators, size
/// and modification time.
fn file_key(path: &Path, metadata: &std::fs::Metadata) -> FileKey {
    let normal: PathBuf = path
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect();
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos() as i128);
    (normal, metadata.len(), mtime_ns)
}

/// The MD5 (32 lowercase hex digits) of the regular file `path`, whose own
/// metadata is `metadata`; `read` reads the file when no digest answers.
pub fn md5_cached(
    path: &Path,
    metadata: &std::fs::Metadata,
    reads: LocalReads<'_>,
    stop: &AtomicBool,
    read: impl FnOnce() -> io::Result<String>,
) -> io::Result<String> {
    let cached = || {
        let stamp = super::local_platform::path_stamp(path);
        let after = || super::local_platform::path_stamp(path);
        if reads.fresh {
            super::hash_cache::digest_fresh(stamp, digest::MD5, read, after)
        } else {
            super::hash_cache::digest(stamp, digest::MD5, read, after)
        }
    };
    match reads.run {
        Some(run) => run.once(file_key(path, metadata), stop, cached),
        None => cached(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn abgleich_task_file_digest_fresh_reads_every_time_and_a_run_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let stop = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let read = || {
            reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("d41d8cd98f00b204e9800998ecf8427e".to_string())
        };
        let fresh = LocalReads {
            fresh: true,
            run: None,
        };
        md5_cached(&path, &metadata, fresh, &stop, read).unwrap();
        md5_cached(&path, &metadata, fresh, &stop, read).unwrap();
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Within one run the second ask takes the first one's digest, also
        // when the file path is spelled differently.
        let run = RunDigests::default();
        let shared = LocalReads {
            fresh: true,
            run: Some(&run),
        };
        let respelled = dir.path().join(".").join("a.bin");
        md5_cached(&path, &metadata, shared, &stop, read).unwrap();
        md5_cached(&respelled, &metadata, shared, &stop, read).unwrap();
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
