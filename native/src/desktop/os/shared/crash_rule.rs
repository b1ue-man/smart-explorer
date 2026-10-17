//! Restart rule of the shell for its surfaces (C5.5): waits of 1 s, 5 s and
//! 30 s along a crash streak; three crashes within ten minutes halt the shell
//! (a crash loop must not burn CPU; one crash heals without the user). The
//! streak starts again after ten minutes without a crash. Exit codes 75
//! (update stop) and 76 (channel end) are never crashes; the caller does not
//! record them.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(crate) const RESTART_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];
pub(crate) const CRASH_WINDOW: Duration = Duration::from_secs(600);
pub(crate) const HALT_AFTER: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CrashVerdict {
    RestartAfter(Duration),
    Halt,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CrashRule {
    window: VecDeque<Instant>,
    streak: usize,
    last: Option<Instant>,
}

impl CrashRule {
    /// Records a crash at `at` and decides what follows.
    pub(crate) fn record(&mut self, at: Instant) -> CrashVerdict {
        if self
            .last
            .is_some_and(|last| at.saturating_duration_since(last) >= CRASH_WINDOW)
        {
            self.streak = 0;
        }
        self.last = Some(at);
        self.streak += 1;
        self.window.push_back(at);
        while self
            .window
            .front()
            .is_some_and(|first| at.saturating_duration_since(*first) >= CRASH_WINDOW)
        {
            self.window.pop_front();
        }
        if self.window.len() >= HALT_AFTER {
            return CrashVerdict::Halt;
        }
        let index = self.streak.clamp(1, RESTART_DELAYS.len()) - 1;
        CrashVerdict::RestartAfter(RESTART_DELAYS[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_rule_waits_then_halts_within_ten_minutes() {
        let start = Instant::now();
        let mut rule = CrashRule::default();
        assert_eq!(
            rule.record(start),
            CrashVerdict::RestartAfter(Duration::from_secs(1))
        );
        assert_eq!(
            rule.record(start + Duration::from_secs(2)),
            CrashVerdict::RestartAfter(Duration::from_secs(5))
        );
        assert_eq!(
            rule.record(start + Duration::from_secs(8)),
            CrashVerdict::Halt
        );

        // Spread crashes never halt; the streak uses the third wait.
        let mut rule = CrashRule::default();
        rule.record(start);
        rule.record(start + Duration::from_secs(300));
        assert_eq!(
            rule.record(start + Duration::from_secs(700)),
            CrashVerdict::RestartAfter(Duration::from_secs(30))
        );
        // Ten quiet minutes start a new streak.
        assert_eq!(
            rule.record(start + Duration::from_secs(1400)),
            CrashVerdict::RestartAfter(Duration::from_secs(1))
        );
    }
}
