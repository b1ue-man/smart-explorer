//! The restart after the end of a surface's channel (exit code 76), part of
//! `supervisor.rs`. A compositor that refuses the surface for good ends the
//! layer process with 76 again and again; the shell then waits longer each
//! time instead of starting it every second: 1, 2, 4, 8, 16 and 32 seconds,
//! then 60 seconds each time. That is a growing wait, not a number of tries:
//! the shell keeps trying for as long as it runs. A surface that ran again
//! (longer than the shell waits for a sign of life, [`super::ALIVE_TIMEOUT`])
//! sets the wait back to one second. The wait is kept per surface key.
use std::collections::HashMap;
use std::time::Duration;

/// The longest wait.
pub(crate) const LONGEST: Duration = Duration::from_secs(60);

/// The wait after `streak` earlier ends in a row: 1, 2, 4, 8, 16, 32 seconds,
/// then [`LONGEST`].
pub(crate) fn delay_for(streak: u32) -> Duration {
    if streak >= 6 {
        LONGEST
    } else {
        Duration::from_secs(1u64 << streak)
    }
}

/// The ends in a row of one surface.
#[derive(Debug, Default)]
pub(crate) struct Backoff {
    streak: u32,
}

impl Backoff {
    /// The wait after an end of the surface that ran for `ran_for`.
    pub(crate) fn next(&mut self, ran_for: Duration) -> Duration {
        if ran_for >= super::ALIVE_TIMEOUT {
            self.streak = 0;
        }
        let wait = delay_for(self.streak);
        self.streak = self.streak.saturating_add(1);
        wait
    }
}

/// One [`Backoff`] per surface key.
#[derive(Debug, Default)]
pub(crate) struct Backoffs {
    by_key: HashMap<String, Backoff>,
}

impl Backoffs {
    pub(crate) fn next(&mut self, key: &str, ran_for: Duration) -> Duration {
        self.by_key
            .entry(key.to_string())
            .or_default()
            .next(ran_for)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_task_surface_restart_backoff_grows() {
        let mut backoff = Backoff::default();
        let short = Duration::from_millis(300);
        let waits: Vec<u64> = (0..9).map(|_| backoff.next(short).as_secs()).collect();
        assert_eq!(
            waits,
            [1, 2, 4, 8, 16, 32, 60, 60, 60],
            "no maximum of tries"
        );
        // A surface that ran for a while starts the sequence again.
        let ran = super::super::ALIVE_TIMEOUT;
        assert_eq!(backoff.next(ran).as_secs(), 1);
        assert_eq!(backoff.next(short).as_secs(), 2);
        // Shorter than that does not count as running.
        let almost = ran - Duration::from_millis(1);
        assert_eq!(backoff.next(almost).as_secs(), 4);
        // Another key has its own wait.
        let mut keyed = Backoffs::default();
        assert_eq!(keyed.next("layer", short).as_secs(), 1);
        assert_eq!(keyed.next("layer", short).as_secs(), 2);
        assert_eq!(keyed.next("DP-1", short).as_secs(), 1);
        assert_eq!(delay_for(u32::MAX), LONGEST);
    }
}
