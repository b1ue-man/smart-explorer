//! Comparison stage of the Android duplicate search: candidates of a shared
//! size are compared by SHA-256 over size + first + last bytes, those with an
//! equal result by SHA-256 over the whole content, both in parallel. The
//! digests equal the desktop's (`duplicates.rs`); `ring` uses the CPU's
//! SHA-256 instructions where present (ARMv8 crypto extensions, SHA-NI).
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::Ordering;

use rayon::prelude::*;
use ring::digest::{Context, SHA256};

use crate::apptrash::ProtectedAreas;

use super::finder::Issues;
use super::finder_walk::Candidate;
use super::retention::compare_group;
use super::stage::ReclaimPhase;
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimConfidence, ReclaimItem,
    ReclaimProgress,
};
use super::util::{hex_lower, to_fwd};

/// Read buffer per worker: the largest FUSE read on Android (`max_read`).
const BUFFER_BYTES: usize = 1024 * 1024;

type Digest = [u8; 32];

pub(super) struct Compare<'a> {
    pub(super) pool: Option<&'a rayon::ThreadPool>,
    pub(super) progress: &'a ReclaimProgress,
    pub(super) protected: &'a [ProtectedAreas],
    pub(super) issues: &'a Issues,
    pub(super) sample_bytes: u64,
    pub(super) roots: &'a [(std::path::PathBuf, crate::local_access::DirectoryHandle)],
}

/// All duplicate groups among `candidates` (largest reclaimable space first)
/// and how many candidates shared their size with another one.
pub(super) fn compare_candidates(
    mut candidates: Vec<Candidate>,
    compare: &Compare<'_>,
) -> (Vec<DuplicateGroup>, u64) {
    let progress = compare.progress;
    candidates.sort_unstable_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.path.cmp(&right.path))
    });
    let same_size = keep_shared(candidates, |candidate| candidate.size);
    let compared = same_size.len() as u64;

    progress
        .stage
        .begin(ReclaimPhase::Fingerprinting, compared, 0);
    let prints = compare.each(&same_size, |candidate, buffer| {
        let result = fingerprint(
            compare.open(candidate),
            candidate.size,
            compare.sample_bytes,
            buffer,
            progress,
        );
        progress.fingerprinted.fetch_add(1, Ordering::Relaxed);
        compare.settle(&candidate.path, "Anfang/Ende", result)
    });
    let mut printed = pair(same_size, prints);
    printed.sort_unstable_by(|(left, left_print), (right, right_print)| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left_print.cmp(right_print))
            .then_with(|| left.path.cmp(&right.path))
    });
    let same_print: Vec<Candidate> =
        keep_shared(printed, |(candidate, print)| (candidate.size, *print))
            .into_iter()
            .map(|(candidate, _)| candidate)
            .collect();
    if progress.cancel.load(Ordering::Relaxed) {
        return (Vec::new(), compared);
    }

    let total_bytes = same_print.iter().fold(0u64, |total, candidate| {
        total.saturating_add(candidate.size)
    });
    progress
        .stage
        .begin(ReclaimPhase::Hashing, same_print.len() as u64, total_bytes);
    let hashes = compare.each(&same_print, |candidate, buffer| {
        let result = content_hash(compare.open(candidate), candidate.size, buffer, progress);
        progress.hashed.fetch_add(1, Ordering::Relaxed);
        compare.settle(&candidate.path, "Inhalt", result)
    });
    if progress.cancel.load(Ordering::Relaxed) {
        return (Vec::new(), compared);
    }
    progress.stage.begin(ReclaimPhase::Grouping, 0, 0);
    let mut hashed = pair(same_print, hashes);
    hashed.sort_unstable_by(|(left, left_hash), (right, right_hash)| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left_hash.cmp(right_hash))
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut groups = Vec::new();
    let mut run: Vec<(Candidate, Digest)> = Vec::new();
    for item in hashed {
        if run
            .last()
            .is_some_and(|(last, hash)| (last.size, *hash) != (item.0.size, item.1))
        {
            groups.extend(group(std::mem::take(&mut run), progress));
        }
        run.push(item);
    }
    groups.extend(group(run, progress));
    groups.sort_by(compare_group);
    (groups, compared)
}

impl Compare<'_> {
    fn open(&self, candidate: &Candidate) -> io::Result<std::fs::File> {
        let (root, handle) = self
            .roots
            .get(candidate.root_index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Kandidatenwurzel fehlt"))?;
        let rel = candidate
            .path
            .strip_prefix(root)
            .map_err(io::Error::other)?;
        let mut directory = handle.clone();
        let mut parts = rel.components().peekable();
        while let Some(part) = parts.next() {
            let std::path::Component::Normal(name) = part else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Kandidat außerhalb der Wurzel",
                ));
            };
            if parts.peek().is_none() {
                return directory.open_regular_child(name);
            }
            directory = directory.open_child(name)?;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Kandidat ist eine Wurzel",
        ))
    }

    /// Runs `work` for every candidate, on the search's pool when it has one,
    /// with one read buffer per worker.
    fn each(
        &self,
        items: &[Candidate],
        work: impl Fn(&Candidate, &mut [u8]) -> Option<Digest> + Sync,
    ) -> Vec<Option<Digest>> {
        match self.pool {
            Some(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map_init(
                        || vec![0u8; BUFFER_BYTES],
                        |buffer, item| work(item, buffer),
                    )
                    .collect()
            }),
            None => {
                let mut buffer = vec![0u8; BUFFER_BYTES];
                items.iter().map(|item| work(item, &mut buffer)).collect()
            }
        }
    }

    /// A failed read drops the candidate: inside a protected area as an
    /// omission, elsewhere as a bounded error.
    fn settle(
        &self,
        path: &Path,
        what: &str,
        result: io::Result<Option<Digest>>,
    ) -> Option<Digest> {
        match result {
            Ok(digest) => digest,
            Err(error) => {
                self.issues.failed(self.protected, path, false, || {
                    format!("{what} {}: {error}", to_fwd(path))
                });
                None
            }
        }
    }
}

/// Runs of at least two equal keys of an already sorted list.
fn keep_shared<T, K: PartialEq>(sorted: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut kept = Vec::with_capacity(sorted.len());
    let mut run: Vec<T> = Vec::new();
    for item in sorted {
        if run.last().is_some_and(|last| key(last) != key(&item)) {
            if run.len() > 1 {
                kept.append(&mut run);
            }
            run.clear();
        }
        run.push(item);
    }
    if run.len() > 1 {
        kept.append(&mut run);
    }
    kept
}

fn pair(candidates: Vec<Candidate>, digests: Vec<Option<Digest>>) -> Vec<(Candidate, Digest)> {
    candidates
        .into_iter()
        .zip(digests)
        .filter_map(|(candidate, digest)| Some((candidate, digest?)))
        .collect()
}

fn group(run: Vec<(Candidate, Digest)>, progress: &ReclaimProgress) -> Option<DuplicateGroup> {
    if run.len() < 2 {
        return None;
    }
    let size = run[0].0.size;
    let hash = hex_lower(&run[0].1);
    let mut items: Vec<ReclaimItem> = run
        .into_iter()
        .map(|(candidate, _)| {
            let name = candidate
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            ReclaimItem::new(
                to_fwd(&candidate.path),
                name,
                candidate.size,
                candidate.mtime_ms,
                false,
            )
            .with_reason("Duplikat", ReclaimConfidence::HashMatch)
        })
        .collect();
    items.sort_by(|left, right| {
        right
            .mtime_ms
            .cmp(&left.mtime_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    progress
        .candidates
        .fetch_add(items.len() as u64, Ordering::Relaxed);
    Some(DuplicateGroup {
        hash: ContentHash {
            algorithm: HashAlgorithm::Sha256,
            hex: hash,
        },
        evidence: DuplicateEvidence::LocalSha256,
        size,
        reclaimable: size.saturating_mul(items.len() as u64 - 1),
        items,
    })
}

fn finish(context: Context) -> Digest {
    let mut digest = [0u8; 32];
    // SHA-256 output is always 32 bytes.
    digest.copy_from_slice(context.finish().as_ref());
    digest
}

fn canceled(progress: &ReclaimProgress) -> bool {
    progress.cancel.load(Ordering::Relaxed)
}

/// SHA-256 over the big-endian size, the first and the last `sample` bytes
/// (the whole file when it is that small), as the desktop's fingerprint.
fn fingerprint(
    file: io::Result<std::fs::File>,
    size: u64,
    sample: u64,
    buffer: &mut [u8],
    progress: &ReclaimProgress,
) -> io::Result<Option<Digest>> {
    if canceled(progress) {
        return Ok(None);
    }
    let mut file = file?;
    let before = file.metadata()?;
    if before.len() != size {
        return Err(changed());
    }
    let sample = sample.max(1).min(size).min(buffer.len() as u64) as usize;
    let mut context = Context::new(&SHA256);
    context.update(&size.to_be_bytes());
    if sample > 0 {
        let buffer = &mut buffer[..sample];
        file.read_exact(buffer)?;
        context.update(buffer);
        if size > sample as u64 {
            if canceled(progress) {
                return Ok(None);
            }
            file.seek(SeekFrom::Start(size - sample as u64))?;
            file.read_exact(buffer)?;
            context.update(buffer);
        }
    }
    unchanged(&file, &before, size)?;
    Ok(Some(finish(context)))
}

/// SHA-256 over the whole content; a file whose length no longer matches the
/// walk is reported as changed instead of being grouped.
fn content_hash(
    file: io::Result<std::fs::File>,
    size: u64,
    buffer: &mut [u8],
    progress: &ReclaimProgress,
) -> io::Result<Option<Digest>> {
    let mut file = file?;
    let before = file.metadata()?;
    if before.len() != size {
        return Err(changed());
    }
    let mut context = Context::new(&SHA256);
    let mut read = 0u64;
    loop {
        if canceled(progress) {
            return Ok(None);
        }
        let count = match file.read(buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        context.update(&buffer[..count]);
        read = read.saturating_add(count as u64);
        progress.stage.add_bytes(count as u64);
        if read > size {
            break;
        }
    }
    if read != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei hat sich während der Suche geändert",
        ));
    }
    unchanged(&file, &before, size)?;
    Ok(Some(finish(context)))
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Datei hat sich während der Suche geändert",
    )
}
fn unchanged(file: &std::fs::File, before: &std::fs::Metadata, size: u64) -> io::Result<()> {
    let after = file.metadata()?;
    if after.len() != size || before.modified().ok() != after.modified().ok() {
        return Err(changed());
    }
    Ok(())
}
