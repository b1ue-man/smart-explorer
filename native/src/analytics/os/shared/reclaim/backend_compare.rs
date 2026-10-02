//! Content comparison of remote duplicate candidates that carry no hash
//! (Share peers without the host-side search, FTP, SMB, WebDAV without
//! checksums, SFTP without agent). Only candidates of a shared size are read:
//! first their ends (SHA-256 over the size and the first and last 64 KiB, as
//! the local search), then – only for equal ends – their whole content. A
//! file never shares its size with another one is never read; reads run in
//! parallel up to the backend's own width.
use std::io::{self, Read};
use std::sync::atomic::Ordering;

use rayon::prelude::*;
use ring::digest::{Context, SHA256};

use super::finder::SAMPLE_BYTES;
use super::retention::compare_group;
use super::stage::ReclaimPhase;
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimConfidence, ReclaimItem,
    ReclaimProgress,
};
use super::util::hex_lower;
use crate::vfs::Backend;

/// Read buffer per worker.
const BUFFER_BYTES: usize = 256 * 1024;
/// More concurrent reads than this never sped up one remote connection.
const MAX_READERS: usize = 16;

type Digest = [u8; 32];

/// What the comparison found: groups (largest reclaimable space first), how
/// many candidates shared their size, and the reads that failed.
pub(super) struct Compared {
    pub(super) groups: Vec<DuplicateGroup>,
    pub(super) compared: u64,
    pub(super) errors: Vec<String>,
}

/// Compares `candidates` (files without a content hash) of `backend`.
pub(super) fn compare_by_content(
    backend: &dyn Backend,
    mut candidates: Vec<ReclaimItem>,
    progress: &ReclaimProgress,
) -> Compared {
    candidates.sort_unstable_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.path.cmp(&right.path))
    });
    let same_size = keep_shared(candidates, |item| item.size);
    let compared = same_size.len() as u64;
    let mut errors = Vec::new();
    let readers = Readers::new(backend.parallelism());

    progress
        .stage
        .begin(ReclaimPhase::Fingerprinting, compared, 0);
    let ends = readers.each(&same_size, |item, buffer| {
        let result = ends_digest(backend, item, buffer, progress);
        progress.fingerprinted.fetch_add(1, Ordering::Relaxed);
        result
    });
    let mut printed = settle(same_size, ends, "Anfang/Ende", &mut errors);
    if canceled(progress) {
        return Compared::empty(compared, errors);
    }
    printed.sort_unstable_by(|(left, left_ends), (right, right_ends)| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left_ends.digest.cmp(&right_ends.digest))
            .then_with(|| left.path.cmp(&right.path))
    });
    let same_ends = keep_shared(printed, |(item, ends)| (item.size, ends.digest));

    // Ends that cover the whole file already are its content.
    let (whole, partial): (Vec<_>, Vec<_>) =
        same_ends.into_iter().partition(|(_, ends)| ends.whole);
    let mut hashed: Vec<(ReclaimItem, Digest)> = whole
        .into_iter()
        .map(|(item, ends)| (item, ends.digest))
        .collect();
    let partial: Vec<ReclaimItem> = partial.into_iter().map(|(item, _)| item).collect();
    let total_bytes = partial
        .iter()
        .fold(0u64, |total, item| total.saturating_add(item.size));
    progress
        .stage
        .begin(ReclaimPhase::Hashing, partial.len() as u64, total_bytes);
    let contents = readers.each(&partial, |item, buffer| {
        let result = content_digest(backend, item, buffer, progress);
        progress.hashed.fetch_add(1, Ordering::Relaxed);
        result
    });
    hashed.extend(settle(partial, contents, "Inhalt", &mut errors));
    if canceled(progress) {
        return Compared::empty(compared, errors);
    }
    progress.stage.begin(ReclaimPhase::Grouping, 0, 0);
    hashed.sort_unstable_by(|(left, left_hash), (right, right_hash)| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left_hash.cmp(right_hash))
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut groups = Vec::new();
    let mut run: Vec<(ReclaimItem, Digest)> = Vec::new();
    for entry in hashed {
        if run
            .last()
            .is_some_and(|(last, hash)| (last.size, *hash) != (entry.0.size, entry.1))
        {
            groups.extend(group(std::mem::take(&mut run), progress));
        }
        run.push(entry);
    }
    groups.extend(group(run, progress));
    groups.sort_by(compare_group);
    Compared {
        groups,
        compared,
        errors,
    }
}

impl Compared {
    fn empty(compared: u64, errors: Vec<String>) -> Self {
        Self {
            groups: Vec::new(),
            compared,
            errors,
        }
    }
}

/// The digest of a file's ends and whether they cover all of it.
#[derive(Clone, Copy)]
struct Ends {
    digest: Digest,
    whole: bool,
}

/// Workers of one comparison: a pool of the backend's width, else serial.
struct Readers {
    pool: Option<rayon::ThreadPool>,
}

impl Readers {
    fn new(width: usize) -> Self {
        let threads = width.clamp(1, MAX_READERS);
        let pool = (threads > 1)
            .then(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .ok()
            })
            .flatten();
        Self { pool }
    }

    fn each<T: Send>(
        &self,
        items: &[ReclaimItem],
        work: impl Fn(&ReclaimItem, &mut [u8]) -> io::Result<Option<T>> + Sync,
    ) -> Vec<io::Result<Option<T>>> {
        match &self.pool {
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
}

/// Pairs every item with its result; a failed read drops the item and is
/// reported, a canceled one just drops it.
fn settle<T>(
    items: Vec<ReclaimItem>,
    results: Vec<io::Result<Option<T>>>,
    what: &str,
    errors: &mut Vec<String>,
) -> Vec<(ReclaimItem, T)> {
    let mut kept = Vec::with_capacity(items.len());
    for (item, result) in items.into_iter().zip(results) {
        match result {
            Ok(Some(value)) => kept.push((item, value)),
            Ok(None) => {}
            Err(error) => errors.push(format!("{what} {}: {error}", item.path)),
        }
    }
    kept
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

fn group(run: Vec<(ReclaimItem, Digest)>, progress: &ReclaimProgress) -> Option<DuplicateGroup> {
    if run.len() < 2 {
        return None;
    }
    let size = run[0].0.size;
    let hex = hex_lower(&run[0].1);
    let mut items: Vec<ReclaimItem> = run
        .into_iter()
        .map(|(item, _)| item.with_reason("Duplikat", ReclaimConfidence::HashMatch))
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
            hex,
        },
        evidence: DuplicateEvidence::LocalSha256,
        size,
        reclaimable: size.saturating_mul(items.len() as u64 - 1),
        items,
    })
}

fn canceled(progress: &ReclaimProgress) -> bool {
    progress.cancel.load(Ordering::Relaxed)
}

/// A reader of `item` from `offset`; `None` when the backend cannot start
/// mid-file (offset 0 always opens).
fn open_at(
    backend: &dyn Backend,
    item: &ReclaimItem,
    offset: u64,
) -> io::Result<Option<Box<dyn Read + Send>>> {
    let id = item.backend_id.as_deref();
    match backend.open_read_at(&item.path, id, offset)? {
        Some(reader) => Ok(Some(reader)),
        None if offset == 0 => backend.open_read_id(&item.path, id).map(Some),
        None => Ok(None),
    }
}

/// SHA-256 over the big-endian size, the first and the last 64 KiB (the
/// whole file when it is that small), equal to the local search's. A
/// backend that cannot read from an offset gets the first bytes only.
fn ends_digest(
    backend: &dyn Backend,
    item: &ReclaimItem,
    buffer: &mut [u8],
    progress: &ReclaimProgress,
) -> io::Result<Option<Ends>> {
    if canceled(progress) {
        return Ok(None);
    }
    let sample = SAMPLE_BYTES.max(1).min(item.size).min(buffer.len() as u64) as usize;
    let mut context = Context::new(&SHA256);
    context.update(&item.size.to_be_bytes());
    if sample == 0 {
        return Ok(Some(Ends {
            digest: finish(context),
            whole: true,
        }));
    }
    let head = open_at(backend, item, 0)?.ok_or_else(unreadable)?;
    read_sample(head, &mut buffer[..sample])?;
    context.update(&buffer[..sample]);
    let tail_at = item.size - sample as u64;
    if tail_at == 0 {
        return Ok(Some(Ends {
            digest: finish(context),
            whole: true,
        }));
    }
    if canceled(progress) {
        return Ok(None);
    }
    let Some(tail) = open_at(backend, item, tail_at)? else {
        return Ok(Some(Ends {
            digest: finish(context),
            whole: false,
        }));
    };
    read_sample(tail, &mut buffer[..sample])?;
    context.update(&buffer[..sample]);
    Ok(Some(Ends {
        digest: finish(context),
        // Head and tail meet or overlap: every byte was read.
        whole: item.size <= 2 * sample as u64,
    }))
}

/// Fills `buffer` from the start of `reader` and closes it (the rest of the
/// stream is canceled).
fn read_sample(mut reader: Box<dyn Read + Send>, buffer: &mut [u8]) -> io::Result<()> {
    reader.read_exact(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            changed()
        } else {
            error
        }
    })
}

/// SHA-256 over the whole content; a length other than the listed one is a
/// file that changed during the search.
fn content_digest(
    backend: &dyn Backend,
    item: &ReclaimItem,
    buffer: &mut [u8],
    progress: &ReclaimProgress,
) -> io::Result<Option<Digest>> {
    if canceled(progress) {
        return Ok(None);
    }
    let mut reader = open_at(backend, item, 0)?.ok_or_else(unreadable)?;
    let mut context = Context::new(&SHA256);
    let mut read = 0u64;
    loop {
        if canceled(progress) {
            return Ok(None);
        }
        let count = match reader.read(buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        context.update(&buffer[..count]);
        read = read.saturating_add(count as u64);
        progress.stage.add_bytes(count as u64);
        if read > item.size {
            break;
        }
    }
    if read != item.size {
        return Err(changed());
    }
    Ok(Some(finish(context)))
}

fn finish(context: Context) -> Digest {
    let mut digest = [0u8; 32];
    // SHA-256 output is always 32 bytes.
    digest.copy_from_slice(context.finish().as_ref());
    digest
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Datei hat sich während der Suche geändert",
    )
}

fn unreadable() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "Datei ist nicht lesbar")
}
