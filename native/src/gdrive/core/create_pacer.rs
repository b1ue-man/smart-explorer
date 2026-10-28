//! Pacing of new Drive objects per account (G3-08a, 2026-10-28). Drive
//! accepts only a few new files per second and account (`transfer_hint`);
//! beyond that it answers 403/429, the worker waits out the refusal and the
//! engine's flow halves. The pacer learns the account's rate instead of
//! assuming one: it does not pace at all until Drive refuses a create, then
//! spaces creates at twice the interval the recent successes came at (half
//! their rate), and shortens the interval by a sixteenth with every success
//! (rate probing upward), doubling it again at the next refusal. No fixed
//! rate, no timer: only Drive's own answers move it.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Successes the observed rate is measured over.
const RECENT: usize = 32;
/// The longest spacing, the longest wait a peer may ask for
/// (`engine_policy::RETRY_AFTER_MAX`).
const MAX_INTERVAL: Duration = Duration::from_secs(60);
/// Below this spacing the pacer steps aside again.
const NO_PACING: Duration = Duration::from_micros(500);

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
    /// How long the next create waits for its slot (and takes the slot).
    pub(super) fn slot(&self) -> Duration {
        self.slot_at(Instant::now())
    }

    fn slot_at(&self, now: Instant) -> Duration {
        let mut state = lock(&self.state);
        let Some(interval) = state.interval else {
            return Duration::ZERO;
        };
        let at = state.next.map_or(now, |next| next.max(now));
        state.next = Some(at + interval);
        at.saturating_duration_since(now)
    }

    /// Waits for the next create's slot.
    pub(super) fn wait(&self) {
        let delay = self.slot();
        if !delay.is_zero() {
            std::thread::sleep(delay);
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
}
