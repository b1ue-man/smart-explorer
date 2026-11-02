//! Prefetching for Explorer, which requests file contents one after another:
//! the files after the one it reads are fetched in parallel, in list order,
//! so small files do not each wait for their own round trips. The flow of
//! the connection decides how many run at once; memory is reserved first,
//! and only when it is free: a file Explorer waits for always comes first.
use super::catalog::Catalog;
use super::fetch::{BufferPlan, Fetch, FetchHandle};
use super::handoff::Handoff;
use super::producer::{spawn_producer, Grant};
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
/// Explorer reads within milliseconds while it copies, even inside one long
/// file; half a minute without any read or request means it waits for the
/// user (a conflict dialog) or stopped. Prefetched files then go back to the
/// shared budget and are fetched again once Explorer continues.
pub(super) const IDLE_RELEASE: Duration = Duration::from_secs(30);

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
    last_activity: Option<Instant>,
    /// Fetches Explorer waits for that wait for memory: no prefetching then.
    demand_waiting: usize,
}

impl Window {
    fn remove(&mut self, index: usize) -> Option<FetchHandle> {
        let handle = self.slots.remove(&index)?;
        self.held = self.held.saturating_sub(handle.reserved());
        Some(handle)
    }

    /// Drops every waiting prefetch; they are fetched again from the cursor.
    /// The handles are returned so they drop outside the lock.
    fn drain(&mut self) -> BTreeMap<usize, FetchHandle> {
        self.held = 0;
        self.scan = self.cursor;
        std::mem::take(&mut self.slots)
    }

    /// Drops every waiting prefetch; the dispatcher rests until the next
    /// request.
    fn release(&mut self) -> BTreeMap<usize, FetchHandle> {
        self.paused = true;
        self.drain()
    }

    fn idle(&self, limit: Duration) -> bool {
        matches!(self.last_activity, Some(last) if last.elapsed() >= limit)
    }
}

pub(super) struct Prefetcher {
    window: Mutex<Window>,
    changed: Condvar,
    idle_release: Duration,
}

/// While alive, a fetch Explorer waits for has claimed the memory the
/// prefetch held; prefetching resumes when it is dropped.
pub(super) struct MemoryYield<'a>(&'a Prefetcher);

impl Drop for MemoryYield<'_> {
    fn drop(&mut self) {
        self.0.update(|window| {
            window.demand_waiting = window.demand_waiting.saturating_sub(1);
            BTreeMap::new()
        });
    }
}

impl Prefetcher {
    pub(super) fn new(idle_release: Duration) -> Self {
        Self {
            window: Mutex::new(Window::default()),
            changed: Condvar::new(),
            idle_release,
        }
    }

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

    /// Explorer is reading: prefetched files stay while it does.
    pub(super) fn touch(&self) {
        self.lock().last_activity = Some(Instant::now());
    }

    /// Explorer asks for `index`: returns its prefetched fetch, if usable,
    /// and moves the window past it. Files Explorer went past (skipped after
    /// a conflict) are dropped; asked for later, they are fetched again.
    pub(super) fn take(&self, index: usize) -> Option<FetchHandle> {
        let mut taken = None;
        self.update(|window| {
            window.paused = false;
            window.last_activity = Some(Instant::now());
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
        // A prefetch that failed may have failed because of the prefetch
        // itself (parallel load); a fetch on request gives the answer.
        taken.filter(|handle| !handle.has_error())
    }

    /// A fetch Explorer waits for cannot get memory: the prefetched files
    /// (read only after it) give theirs back until the guard drops.
    pub(super) fn yield_memory(&self) -> MemoryYield<'_> {
        self.update(|window| {
            window.demand_waiting += 1;
            window.drain()
        });
        MemoryYield(self)
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
            if !window.slots.is_empty() && window.idle(self.idle_release) {
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
            window = self.wait(window);
        }
    }

    fn wait<'a>(&self, window: MutexGuard<'a, Window>) -> MutexGuard<'a, Window> {
        match self.changed.wait_timeout(window, WAIT_SLICE) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    fn candidate(
        window: &mut Window,
        catalog: &Catalog,
        flow: &Flow,
        share: u64,
    ) -> Option<(usize, BufferPlan)> {
        if window.demand_waiting > 0 {
            return None;
        }
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

    /// The budget had no room for `index`: try it again after a change or
    /// a wait slice (memory frees without notice).
    fn retry_later(&self, index: usize) {
        let mut window = self.lock();
        window.scan = window.scan.min(index).max(window.cursor);
        drop(self.wait(window));
    }

    /// Adds a started prefetch unless Explorer already passed or took it, or
    /// a fetch it waits for needs the memory.
    fn claim(&self, index: usize, handle: FetchHandle) -> bool {
        let mut window = self.lock();
        if window.closed
            || window.paused
            || window.demand_waiting > 0
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
        // K2: memory first, then the permit. Prefetching never queues for
        // memory, so freed memory goes to fetches Explorer waits for.
        let Some(held) = handoff.memory.try_reserve(plan.reserve()) else {
            prefetcher.retry_later(index);
            continue;
        };
        let Some(permit) = handoff
            .flow
            .acquire_for(handoff.prefetch_job, &handoff.closed)
        else {
            return;
        };
        let Ok(fetch) = Fetch::new(entry.clone(), 0, plan) else {
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
