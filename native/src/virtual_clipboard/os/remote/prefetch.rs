//! Prefetching for Explorer, which requests file contents one after another:
//! the files after the one it reads are fetched in parallel, in list order,
//! so small files do not each wait for their own round trips. The flow of
//! the connection decides how many run at once; memory is reserved first.
use super::catalog::Catalog;
use super::fetch::{spawn_producer, BufferPlan, Fetch, FetchHandle, Grant};
use super::handoff::Handoff;
use super::signal::WAIT_SLICE;
use crate::transfer::Flow;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Finished files waiting for Explorer, per permit of the flow: one flow
/// window in flight plus one window ready. More cannot speed Explorer up (it
/// reads one file at a time) and would only hold shared memory.
const WINDOWS_AHEAD: usize = 2;
/// Explorer asks for the next file within milliseconds while it copies; half
/// a minute without a request means it waits for the user (a conflict
/// dialog) or stopped. Prefetched files then go back to the shared budget and
/// are fetched again once Explorer continues.
const IDLE_RELEASE: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Window {
    running: bool,
    closed: bool,
    /// No prefetching until Explorer asks for a file again.
    paused: bool,
    /// Explorer's next index in list order.
    cursor: usize,
    /// Next index the dispatcher considers.
    scan: usize,
    slots: BTreeMap<usize, FetchHandle>,
    /// Budget bytes the slots hold.
    held: u64,
    last_request: Option<Instant>,
}

impl Window {
    fn remove(&mut self, index: usize) -> Option<FetchHandle> {
        let handle = self.slots.remove(&index)?;
        self.held = self.held.saturating_sub(handle.reserved());
        Some(handle)
    }

    /// Drops every waiting prefetch; the dispatcher rests until the next
    /// request. The handles are returned so they drop outside the lock.
    fn release(&mut self) -> BTreeMap<usize, FetchHandle> {
        self.paused = true;
        self.held = 0;
        self.scan = self.cursor;
        std::mem::take(&mut self.slots)
    }

    fn idle(&self) -> bool {
        matches!(self.last_request, Some(last) if last.elapsed() >= IDLE_RELEASE)
    }
}

#[derive(Default)]
pub(super) struct Prefetcher {
    window: Mutex<Window>,
    changed: Condvar,
}

impl Prefetcher {
    fn lock(&self) -> MutexGuard<'_, Window> {
        self.window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Applies `change`; the prefetches it removes stop outside the lock.
    fn update(&self, change: impl FnOnce(&mut Window) -> BTreeMap<usize, FetchHandle>) {
        let dropped = {
            let mut window = self.lock();
            change(&mut window)
        };
        drop(dropped);
        self.changed.notify_all();
    }

    /// Explorer asks for `index`: returns its prefetched fetch, if usable,
    /// and moves the window past it. Files Explorer went past (skipped after
    /// a conflict) are dropped; asked for later, they are fetched again.
    pub(super) fn take(&self, index: usize) -> Option<FetchHandle> {
        let mut taken = None;
        self.update(|window| {
            window.paused = false;
            window.last_request = Some(Instant::now());
            window.cursor = window.cursor.max(index.saturating_add(1));
            taken = window.remove(index);
            let cursor = window.cursor;
            let kept = window.slots.split_off(&cursor);
            let stale = std::mem::replace(&mut window.slots, kept);
            let freed: u64 = stale.values().map(FetchHandle::reserved).sum();
            window.held = window.held.saturating_sub(freed);
            window.scan = window.scan.max(cursor);
            stale
        });
        // Overload came from our own parallelism: fetch again on demand
        // instead of showing Explorer an error.
        taken.filter(|handle| !handle.failed_with_overload())
    }

    /// Starts the dispatcher once Explorer reads a file.
    pub(super) fn ensure_running(&self, handoff: &Arc<Handoff>, catalog: &Arc<Catalog>) {
        {
            let mut window = self.lock();
            if window.running || window.closed {
                return;
            }
            window.running = true;
        }
        let (handoff, catalog) = (handoff.clone(), catalog.clone());
        let spawned = std::thread::Builder::new()
            .name("remote-prefetch".into())
            .spawn(move || dispatch(&handoff, &catalog));
        if spawned.is_err() {
            // Without prefetching every file is still fetched on request.
            self.lock().running = false;
        }
    }

    /// A new paste of the same object starts over at the first file, once
    /// Explorer asks for it.
    pub(super) fn reset(&self) {
        self.update(|window| {
            window.cursor = 0;
            window.release()
        });
    }

    /// Explorer ended its copy: nothing more is read in this paste.
    pub(super) fn release(&self) {
        self.update(Window::release);
    }

    pub(super) fn close(&self) {
        self.update(|window| {
            window.closed = true;
            window.release()
        });
    }

    /// The next file to prefetch, once the window has room for it; `None`
    /// when the hand-off ended or nothing is left to do until Explorer asks
    /// again (the dispatcher then ends; the next request starts it again).
    fn next(
        &self,
        catalog: &Catalog,
        flow: &Flow,
        share: u64,
        closed: &AtomicBool,
    ) -> Option<(usize, BufferPlan)> {
        let mut window = self.lock();
        loop {
            if window.closed || closed.load(Ordering::Acquire) {
                return None;
            }
            if !window.slots.is_empty() && window.idle() {
                let stale = window.release();
                drop(stale);
            }
            window.scan = window.scan.max(window.cursor);
            if window.paused || window.scan >= catalog.entries.len() {
                if window.slots.is_empty() {
                    window.running = false;
                    return None;
                }
            } else if let Some(next) = Self::candidate(&mut window, catalog, flow, share) {
                return Some(next);
            }
            window = match self.changed.wait_timeout(window, WAIT_SLICE) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    fn candidate(
        window: &mut Window,
        catalog: &Catalog,
        flow: &Flow,
        share: u64,
    ) -> Option<(usize, BufferPlan)> {
        let ahead = WINDOWS_AHEAD * flow.snapshot().limit.max(1);
        while window.slots.len() < ahead {
            let index = window.scan;
            let entry = catalog.entries.get(index)?;
            if !BufferPlan::prefetchable(entry) || window.slots.contains_key(&index) {
                window.scan += 1;
                continue;
            }
            let plan = BufferPlan::for_entry(entry);
            // Half the budget at most, so the file Explorer is waiting for
            // (and other transfers) never wait for prefetched files Explorer
            // reads only after it.
            if !window.slots.is_empty() && window.held + plan.reserve() > share {
                return None;
            }
            window.scan += 1;
            return Some((index, plan));
        }
        None
    }

    /// Adds a started prefetch unless Explorer already passed or took it.
    fn claim(&self, index: usize, handle: FetchHandle) -> bool {
        let mut window = self.lock();
        if window.closed
            || window.paused
            || index < window.cursor
            || window.slots.contains_key(&index)
        {
            drop(window);
            drop(handle);
            return false;
        }
        window.held += handle.reserved();
        window.slots.insert(index, handle);
        true
    }
}

fn dispatch(handoff: &Arc<Handoff>, catalog: &Arc<Catalog>) {
    let share = handoff.memory.capacity() / 2;
    let prefetcher = &handoff.prefetch;
    while let Some((index, plan)) = prefetcher.next(catalog, &handoff.flow, share, &handoff.closed)
    {
        let Some(entry) = catalog.entries.get(index) else {
            continue;
        };
        // K2: memory first, then the permit; nothing waits while holding one.
        let Some(held) = handoff.memory.reserve(plan.reserve(), &handoff.closed) else {
            return;
        };
        let Some(permit) = handoff
            .flow
            .acquire_for(handoff.prefetch_job, &handoff.closed)
        else {
            return;
        };
        let Ok(fetch) = Fetch::new(entry.clone(), plan) else {
            permit.abandon();
            continue;
        };
        if !prefetcher.claim(index, FetchHandle::new(fetch.clone())) {
            permit.abandon();
            continue;
        }
        let grant = Grant { held, permit };
        spawn_producer(handoff.clone(), fetch, handoff.prefetch_job, Some(grant));
    }
}
