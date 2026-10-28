//! Parallel extraction lanes (G4 MZ1). The central directory is planned on
//! one thread (names, links, folders: everything that decides where a byte
//! may go); the file entries are then decompressed by one worker per core,
//! each with its own handle on the one parsed archive.
//!
//! Entries that may name the same file on the target go to the same lane in
//! archive order: the lane key ignores letter case and the trailing dots and
//! spaces Windows drops, so a later duplicate still replaces (or, with
//! `create_new`, still refuses) exactly like in a sequential pass.
use super::entry::{Archive, ArchiveFile};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Copy block of one extracted file.
pub(super) const BLOCK: usize = 1024 * 1024;

/// One file entry to write: its archive index and target path.
#[derive(Clone, Debug)]
pub(super) struct Job {
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) target: PathBuf,
}

/// The archive at `path`, parsed once; clones share the parse and open
/// their own handle on first read.
pub(super) fn open(path: &Path) -> io::Result<Archive> {
    let file = std::fs::File::open(path)?;
    zip::ZipArchive::new(ArchiveFile::opened(path.to_path_buf(), file))
        .map_err(|error| io::Error::other(error.to_string()))
}

/// Workers for `jobs` file entries: one per core, never more than entries.
fn workers(jobs: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(jobs)
        .max(1)
}

/// The spelling-independent key of a target path (see the module comment).
fn lane_key(target: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in target.components() {
        let text = part.as_os_str().to_string_lossy().to_lowercase();
        text.trim_end_matches(['.', ' ']).hash(&mut hasher);
    }
    hasher.finish()
}

/// `jobs` split into lanes; each lane keeps archive order.
pub(super) fn lanes(jobs: Vec<Job>) -> Vec<Vec<Job>> {
    let count = workers(jobs.len());
    let mut lanes = vec![Vec::new(); count];
    for job in jobs {
        let lane = (lane_key(&job.target) % count as u64) as usize;
        lanes[lane].push(job);
    }
    lanes.retain(|lane| !lane.is_empty());
    lanes
}

/// Copies `entry` into `output` with 1 MiB blocks; `Ok(None)` when `cancel`
/// was set. `copied` sees every block.
pub(super) fn copy(
    entry: &mut dyn Read,
    output: &mut dyn Write,
    cancel: &AtomicBool,
    mut copied: impl FnMut(u64),
) -> io::Result<Option<u64>> {
    let mut buffer = vec![0u8; BLOCK];
    let mut written = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let read = match entry.read(&mut buffer) {
            Ok(0) => return Ok(Some(written)),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        output.write_all(&buffer[..read])?;
        written = written.saturating_add(read as u64);
        copied(read as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(index: usize, target: &str) -> Job {
        Job {
            index,
            name: target.to_string(),
            target: PathBuf::from(target),
        }
    }

    #[test]
    fn speed_task_zip_lanes_keep_same_named_targets_in_archive_order() {
        let jobs = vec![
            job(0, "dir/A.txt"),
            job(1, "other.bin"),
            job(2, "dir/a.txt"),
            job(3, "dir/a.txt. "),
        ];
        let lanes = lanes(jobs);
        let holder = lanes
            .iter()
            .find(|lane| lane.iter().any(|job| job.index == 0))
            .unwrap();
        let order: Vec<usize> = holder
            .iter()
            .filter(|job| job.name.to_lowercase().starts_with("dir/a.txt"))
            .map(|job| job.index)
            .collect();
        assert_eq!(order, [0, 2, 3], "one lane, archive order");
        let total: usize = lanes.iter().map(Vec::len).sum();
        assert_eq!(total, 4);
    }
}
