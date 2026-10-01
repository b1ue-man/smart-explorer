//! Both clocks the idle logic of the Share client reads.
//!
//! `Instant`, Tokio and every QUIC timer run on the monotonic clock, which
//! stands still while an Android device is suspended. The Share server,
//! NAT mappings and the peers count real time, so every idle decision that
//! must survive a suspend (presence lifetime, server silence, quiet peer
//! connections) reads the wall clock as well.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One reading of both clocks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Now {
    pub(crate) mono: Instant,
    pub(crate) wall_ms: i64,
}

impl Now {
    /// Real time since `earlier`; zero when the wall clock went backwards.
    pub(crate) fn wall_since(&self, earlier: &Now) -> Duration {
        Duration::from_millis(
            u64::try_from(self.wall_ms.saturating_sub(earlier.wall_ms)).unwrap_or(0),
        )
    }

    /// Wall time in whole Unix seconds.
    pub(crate) fn unix_secs(&self) -> i64 {
        self.wall_ms.div_euclid(1000)
    }
}

pub(crate) trait PowerClock: Send + Sync {
    fn now(&self) -> Now;

    /// Longest real blocking wait for a computed `timeout`. Production waits
    /// the full time; a test clock caps it, because its monotonic clock only
    /// moves when the test advances it.
    fn real_wait(&self, timeout: Duration) -> Duration {
        timeout
    }
}

pub(crate) struct SystemClock;

impl PowerClock for SystemClock {
    fn now(&self) -> Now {
        Now {
            mono: Instant::now(),
            wall_ms: wall_ms_now(),
        }
    }
}

pub(crate) fn wall_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Test clock: `advance` lets both clocks run (device awake), `suspend` only
/// the wall clock (device asleep, monotonic timers stand still).
#[cfg(test)]
pub(crate) struct ManualClock {
    start: Instant,
    state: std::sync::Mutex<(Duration, i64)>,
}

#[cfg(test)]
impl ManualClock {
    pub(crate) fn new(wall_ms: i64) -> Self {
        Self {
            start: Instant::now(),
            state: std::sync::Mutex::new((Duration::ZERO, wall_ms)),
        }
    }

    pub(crate) fn advance(&self, elapsed: Duration) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.0 += elapsed;
        state.1 += i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    }

    pub(crate) fn suspend(&self, elapsed: Duration) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.1 += i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    }
}

#[cfg(test)]
impl PowerClock for ManualClock {
    fn now(&self) -> Now {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Now {
            mono: self.start + state.0,
            wall_ms: state.1,
        }
    }

    fn real_wait(&self, timeout: Duration) -> Duration {
        timeout.min(Duration::from_millis(20))
    }
}
