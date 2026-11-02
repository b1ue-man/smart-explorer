//! Pacing of new Drive objects per account (G3-08a, 2026-10-28). Drive
//! accepts only a few new files per second and account (`transfer_hint`);
//! beyond that it answers 403/429, the worker waits out the refusal and the
//! engine's flow halves. The pacer learns the account's rate instead of
//! assuming one: it does not pace at all until Drive refuses a create, then
//! spaces creates at twice the interval the recent successes came at (half
//! their rate), and shortens the interval by a sixteenth with every success
//! (rate probing upward), doubling it again at the next refusal. No fixed
//! rate, no timer: only Drive's own answers move it. A wait for a slot ends
//! as soon as the job running on the thread is canceled, and then no create
//! goes out.
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Successes the observed rate is measured over.
const RECENT: usize = 32;
/// The longest spacing, the longest wait a peer may ask for
/// (`engine_policy::RETRY_AFTER_MAX`).
const MAX_INTERVAL: Duration = Duration::from_secs(60);
/// Below this spacing the pacer steps aside again.
const NO_PACING: Duration = Duration::from_micros(500);
/// How often a waiting create looks at its job's cancel flag.
const CANCEL_SLICE: Duration = Duration::from_millis(50);

#[derive(Default)]
struct PaceState {
    interval: Option<Duration>,
    next: Option<Instant>,
    recent: VecDeque<Instant>,
}

#[derive(Default)]
pub(super) struct CreatePacer {
    state: Mutex<PaceState>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pacer of the account `account` (shared by every backend of it).
pub(super) fn for_account(account: &str) -> Arc<CreatePacer> {
    static PACERS: OnceLock<Mutex<HashMap<String, Arc<CreatePacer>>>> = OnceLock::new();
    let pacers = PACERS.get_or_init(|| Mutex::new(HashMap::new()));
    lock(pacers).entry(account.to_string()).or_default().clone()
}

impl CreatePacer {
    #[cfg(test)]
    fn slot_at(&self, now: Instant) -> Duration {
        self.reserve_at(now)
            .map_or(Duration::ZERO, |(at, _)| at.saturating_duration_since(now))
    }

    /// Takes the next create's slot: when it starts and when the one after
    /// it may start (`None` while the pacer steps aside).
    fn reserve_at(&self, now: Instant) -> Option<(Instant, Instant)> {
        let mut state = lock(&self.state);
        let interval = state.interval?;
        let at = state.next.map_or(now, |next| next.max(now));
        let end = at + interval;
        state.next = Some(end);
        Some((at, end))
    }

    /// A slot whose create never went out: the next asker may take it,
    /// unless someone reserved after it already.
    fn give_back(&self, at: Instant, end: Instant) {
        let mut state = lock(&self.state);
        if state.next == Some(end) {
            state.next = Some(at);
        }
    }

    /// Waits for the next create's slot; `Interrupted` (and the slot given
    /// back) once the job running on this thread is canceled
    /// (`source_media::with_cancellation`, set by every transfer worker).
    pub(super) fn wait(&self) -> io::Result<()> {
        self.wait_unless(crate::source_media::context_canceled)
    }

    fn wait_unless(&self, canceled: impl Fn() -> bool) -> io::Result<()> {
        let slot = self.reserve_at(Instant::now());
        loop {
            if canceled() {
                if let Some((at, end)) = slot {
                    self.give_back(at, end);
                }
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Drive-Upload abgebrochen, bevor die Datei angelegt wurde",
                ));
            }
            let now = Instant::now();
            let Some(remaining) = slot
                .map(|(at, _)| at.saturating_duration_since(now))
                .filter(|remaining| !remaining.is_zero())
            else {
                return Ok(());
            };
            std::thread::sleep(remaining.min(CANCEL_SLICE));
        }
    }

    /// Drive accepted a create.
    pub(super) fn created(&self) {
        self.created_at(Instant::now());
    }

    fn created_at(&self, now: Instant) {
        let mut state = lock(&self.state);
        state.recent.push_back(now);
        while state.recent.len() > RECENT {
            state.recent.pop_front();
        }
        if let Some(interval) = state.interval {
            let shorter = interval.saturating_sub(interval / 16);
            state.interval = (shorter >= NO_PACING).then_some(shorter);
        }
    }

    /// Drive refused a create as too fast; `retry_after` when it named a
    /// delay.
    pub(super) fn congested(&self, retry_after: Option<Duration>) {
        let mut state = lock(&self.state);
        let interval = match state.interval {
            Some(interval) => interval.saturating_mul(2),
            None => match observed_interval(&state.recent) {
                Some(observed) => observed.saturating_mul(2),
                None => match retry_after {
                    Some(delay) => delay,
                    // Nothing to learn from yet: the engine's flow decides.
                    None => return,
                },
            },
        };
        state.interval = Some(interval.clamp(NO_PACING, MAX_INTERVAL));
    }
}

/// The mean spacing of the recent successes.
fn observed_interval(recent: &VecDeque<Instant>) -> Option<Duration> {
    let (first, last) = (recent.front()?, recent.back()?);
    let gaps = u32::try_from(recent.len().checked_sub(1)?).ok()?;
    if gaps == 0 {
        return None;
    }
    Some(last.saturating_duration_since(*first) / gaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_drive_pacer_learns_only_from_refusals() {
        let pacer = CreatePacer::default();
        let start = Instant::now();
        for step in 0..10u32 {
            pacer.created_at(start + Duration::from_millis(100) * step);
        }
        assert_eq!(
            pacer.slot_at(start),
            Duration::ZERO,
            "no refusal: no pacing"
        );
        pacer.congested(None);
        // Successes came every 100 ms: half that rate.
        let now = start + Duration::from_secs(1);
        assert_eq!(pacer.slot_at(now), Duration::ZERO, "the first slot is free");
        assert_eq!(pacer.slot_at(now), Duration::from_millis(200));
        pacer.created_at(now);
        let interval = lock(&pacer.state).interval;
        assert_eq!(
            interval,
            Some(Duration::from_micros(187_500)),
            "probes upward"
        );
        pacer.congested(None);
        let interval = lock(&pacer.state).interval;
        assert_eq!(
            interval,
            Some(Duration::from_micros(375_000)),
            "halves again"
        );
    }

    #[test]
    fn speed_task_drive_pacer_wait_ends_at_cancel_and_gives_the_slot_back() {
        let pacer = CreatePacer::default();
        lock(&pacer.state).interval = Some(MAX_INTERVAL);
        // The first slot is free; the second would wait a whole interval.
        pacer.wait_unless(|| false).expect("the free slot");
        let reserved = lock(&pacer.state).next;
        let started = Instant::now();
        let canceled = std::sync::atomic::AtomicBool::new(false);
        let error = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                canceled.store(true, std::sync::atomic::Ordering::Release);
            });
            pacer
                .wait_unless(|| canceled.load(std::sync::atomic::Ordering::Acquire))
                .expect_err("a cancel ends the wait")
        });
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < MAX_INTERVAL, "not the whole interval");
        assert_eq!(
            lock(&pacer.state).next,
            reserved,
            "the canceled create's slot is free again"
        );
        // Canceled before any wait: no create either.
        assert!(pacer.wait_unless(|| true).is_err());
    }
}
