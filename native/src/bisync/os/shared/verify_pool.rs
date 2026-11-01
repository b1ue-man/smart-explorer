//! The content check of the planner (`orchestration_plan::prepare_spelled`):
//! every key the planner cannot decide by size and time (`Decision::Verify`,
//! equal sizes without a usable hash) needs the MD5 of both sides.
//!
//! Both sides are checked at the same time, each by its own workers, so a
//! slow side never holds the faster one back. A remote side that forms MD5s
//! where its files are stored first answers whole folders in one request
//! (`vfs::content_md5_many`, as the walk does); what it leaves open, and
//! every file of a local side, goes to the side's workers one file each.
//!
//! How many workers a stage runs is not fixed. `Pace` measures the stage's
//! throughput in observation windows and doubles the workers while that
//! brings more, halves them when it brings less and then settles in single
//! steps around the best count. A fast SSD, a seeking disk and a link far
//! away each run the parallelism they carry, and the count follows the
//! device while the stage runs.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::types::Sig;
use crate::vfs::{Backend, VfsMeta};

/// One file of one side to check.
pub(super) struct Job {
    /// Index of the key in the planner's `verify` list.
    pub(super) key: usize,
    /// Backend path of the file on this side.
    pub(super) path: String,
    /// What the planner saw; the check binds its MD5 to exactly this.
    pub(super) planned: Sig,
}

/// The MD5 per `Job::key` of one side (`run_side`).
pub(super) type SideResults = Vec<(usize, io::Result<u64>)>;

/// The check of one file: its MD5 as the planner's `u64`, bound to `planned`.
pub(super) type Check<'a> = dyn Fn(&str, Sig) -> io::Result<u64> + Sync + 'a;

/// Shortest observation window: shorter windows measure scheduler noise.
const MIN_WINDOW: Duration = Duration::from_millis(200);
/// Longest observation window: a slow link still adapts this often.
const MAX_WINDOW: Duration = Duration::from_secs(2);
/// A change of the worker count counts only beyond this share of throughput.
const SIGNIFICANT: f64 = 0.05;

/// The MD5 of every job of one side, keyed by `Job::key`; a canceled check
/// leaves the rest without a result, like the former loop did.
pub(super) fn run_side(
    backend: &dyn Backend,
    jobs: Vec<Job>,
    check: &Check<'_>,
    cancel: &AtomicBool,
) -> SideResults {
    let batched = batch(backend, &jobs, cancel);
    let mut results = Vec::with_capacity(jobs.len());
    let mut open = Vec::new();
    for (job, hash) in jobs.into_iter().zip(batched) {
        match hash {
            Some(hash) => results.push((job.key, Ok(hash))),
            None => open.push(job),
        }
    }
    let checked = pool(
        &open,
        |job: &Job| job.planned.size,
        cancel,
        |job| (job.key, check(&job.path, job.planned)),
    );
    results.extend(checked);
    results
}

/// The MD5s a remote side forms where its files are stored, one request per
/// folder; the folders run in their own pool. `None`: the file asks alone.
fn batch(backend: &dyn Backend, jobs: &[Job], cancel: &AtomicBool) -> Vec<Option<u64>> {
    let mut out = vec![None; jobs.len()];
    // Local files are read by the workers; names that are not unique (Drive)
    // cannot be matched to an answer by path.
    if jobs.is_empty() || backend.is_local() || backend.has_duplicate_file_names() {
        return out;
    }
    let mut folders: std::collections::BTreeMap<&str, Vec<usize>> = Default::default();
    for (index, job) in jobs.iter().enumerate() {
        let folder = job.path.rsplit_once('/').map_or("", |(folder, _)| folder);
        folders.entry(folder).or_default().push(index);
    }
    let groups: Vec<Vec<usize>> = folders.into_values().collect();
    let answered = pool(
        &groups,
        |group: &Vec<usize>| group.iter().map(|index| jobs[*index].planned.size).sum(),
        cancel,
        |group| {
            let metas: Vec<VfsMeta> = group
                .iter()
                .map(|index| VfsMeta {
                    size: jobs[*index].planned.size,
                    mtime_ms: jobs[*index].planned.mtime_ms,
                    ..VfsMeta::default()
                })
                .collect();
            let asked: Vec<(&str, &VfsMeta)> = group
                .iter()
                .zip(&metas)
                .map(|(index, meta)| (jobs[*index].path.as_str(), meta))
                .collect();
            // An error or a side without batching leaves every file to its
            // own check, which reports what went wrong.
            let found = crate::vfs::content_md5_many(backend, &asked, cancel)
                .ok()
                .flatten();
            (group.clone(), found)
        },
    );
    for (group, found) in answered {
        let Some(found) = found else { continue };
        for (index, hex) in group.into_iter().zip(found) {
            out[index] = hex
                .map(|hex| super::snapshot::md5_hex_to_u64(&hex))
                .filter(|hash| *hash != 0);
        }
    }
    out
}

/// Runs `work` for every item on a worker pool whose size `Pace` chooses;
/// `weight` is an item's size in bytes. Results come back in no order.
fn pool<T: Sync, R: Send>(
    items: &[T],
    weight: impl Fn(&T) -> u64 + Sync,
    cancel: &AtomicBool,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    if items.is_empty() {
        return Vec::new();
    }
    let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
    let shared = Shared {
        next: AtomicUsize::new(0),
        done_items: AtomicU64::new(0),
        done_bytes: AtomicU64::new(0),
        limit: Mutex::new(cores.min(items.len())),
        wake: Condvar::new(),
    };
    // The workers grow only while throughput grows (`Pace`); this bound keeps
    // their read buffers (`snapshot_hash`, two chunks each) to a few per core.
    let most = (cores * 8).min(items.len());
    let start = *shared.limit();
    let mut pace = Pace::new(start, most);
    std::thread::scope(|scope| {
        let run = |id: usize| {
            let shared = &shared;
            let (work, weight) = (&work, &weight);
            move || {
                let mut out = Vec::new();
                loop {
                    if !shared.admit(id, cancel) {
                        return out;
                    }
                    let index = shared.next.fetch_add(1, Ordering::AcqRel);
                    let Some(item) = items.get(index) else {
                        return out;
                    };
                    out.push(work(item));
                    shared.done_bytes.fetch_add(weight(item), Ordering::AcqRel);
                    shared.done_items.fetch_add(1, Ordering::AcqRel);
                }
            }
        };
        let mut workers: Vec<_> = (0..start).map(|id| scope.spawn(run(id))).collect();
        let mut window = Window::start(&shared);
        while shared.next.load(Ordering::Acquire) < items.len() && !cancel.load(Ordering::Acquire) {
            std::thread::sleep(MIN_WINDOW / 4);
            let running = *shared.limit();
            let Some(score) = window.score(&shared, running) else {
                continue;
            };
            let limit = pace.next(score);
            while workers.len() < limit {
                workers.push(scope.spawn(run(workers.len())));
            }
            *shared.limit() = limit;
            shared.wake.notify_all();
            window = Window::start(&shared);
        }
        // Waiting workers see that nothing is left and end.
        *shared.limit() = workers.len();
        shared.wake.notify_all();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    })
}

struct Shared {
    next: AtomicUsize,
    done_items: AtomicU64,
    done_bytes: AtomicU64,
    /// How many workers may take an item now (`Pace`).
    limit: Mutex<usize>,
    wake: Condvar,
}

impl Shared {
    fn limit(&self) -> MutexGuard<'_, usize> {
        self.limit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits while worker `id` is above the current count; `false` once the
    /// stage is canceled.
    fn admit(&self, id: usize, cancel: &AtomicBool) -> bool {
        let mut limit = self.limit();
        while id >= *limit {
            if cancel.load(Ordering::Acquire) {
                return false;
            }
            limit = match self.wake.wait_timeout(limit, MIN_WINDOW) {
                Ok((limit, _)) => limit,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        !cancel.load(Ordering::Acquire)
    }
}

/// One observation window of a stage.
struct Window {
    since: Instant,
    items: u64,
    bytes: u64,
}

impl Window {
    fn start(shared: &Shared) -> Self {
        Self {
            since: Instant::now(),
            items: shared.done_items.load(Ordering::Acquire),
            bytes: shared.done_bytes.load(Ordering::Acquire),
        }
    }

    /// Throughput of the window once it saw every running worker finish an
    /// item (or ran `MAX_WINDOW`): bytes per second plus files per second
    /// weighted with the stage's mean file size so far, so a window of small
    /// files and one of large files measure the same work.
    fn score(&self, shared: &Shared, running: usize) -> Option<f64> {
        let elapsed = self.since.elapsed();
        let items_total = shared.done_items.load(Ordering::Acquire);
        let bytes_total = shared.done_bytes.load(Ordering::Acquire);
        let (items, bytes) = (items_total - self.items, bytes_total - self.bytes);
        let seen = elapsed >= MIN_WINDOW && items >= running as u64;
        if !seen && elapsed < MAX_WINDOW {
            return None;
        }
        let mean = bytes_total as f64 / items_total.max(1) as f64;
        Some((bytes as f64 + items as f64 * mean) / elapsed.as_secs_f64())
    }
}

/// The worker count of one stage: doubling while the throughput rises,
/// halving back once it falls, then single steps around the best count.
pub(super) struct Pace {
    limit: usize,
    most: usize,
    previous: Option<(usize, f64)>,
    step: usize,
    growing: bool,
}

impl Pace {
    pub(super) fn new(start: usize, most: usize) -> Self {
        let most = most.max(1);
        Self {
            limit: start.clamp(1, most),
            most,
            previous: None,
            step: start.clamp(1, most),
            growing: true,
        }
    }

    /// The count for the next window after a window at `self.limit` workers
    /// measured `score`.
    pub(super) fn next(&mut self, score: f64) -> usize {
        let current = self.limit;
        match self.previous {
            Some((count, before)) if count != current => {
                if score < before * (1.0 - SIGNIFICANT) {
                    // The change cost throughput: go back and step finer.
                    self.growing = current < count;
                    self.step = (self.step / 2).max(1);
                    self.limit = count;
                    self.previous = Some((current, score));
                    return self.limit;
                }
                if score <= before * (1.0 + SIGNIFICANT) {
                    // No gain: keep the smaller count, probe in single steps.
                    self.step = 1;
                    self.limit = current.min(count);
                    self.previous = Some((self.limit, score.max(before)));
                    self.growing = !self.growing;
                    return self.limit;
                }
                if self.step > 1 && self.growing {
                    self.step = (self.step * 2).min(self.most);
                }
            }
            _ => {}
        }
        self.previous = Some((current, score));
        self.limit = if self.growing {
            (current + self.step).min(self.most)
        } else {
            current.saturating_sub(self.step).max(1)
        };
        if self.limit == current {
            self.growing = !self.growing;
        }
        self.limit
    }
}

#[cfg(test)]
#[path = "verify_pool_tests.rs"]
mod tests;
