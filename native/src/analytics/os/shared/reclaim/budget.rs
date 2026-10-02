use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub(super) const MAX_RECLAIM_ENTRIES: u64 = 1_000_000;
pub(super) const MAX_RECLAIM_TEXT_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_RECLAIM_DEPTH: u32 = 512;

pub(super) struct ReclaimBudget {
    entries: u64,
    text_bytes: u64,
    max_entries: u64,
    max_text_bytes: u64,
    max_depth: u32,
    stopped: Option<LimitExceeded>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LimitExceeded {
    limit: &'static str,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bounded {} limit", self.limit)
    }
}

const LIMITS: [&str; 3] = ["depth", "entry count", "path/name text"];

impl LimitExceeded {
    /// German wording for the Android duplicate summary.
    pub(super) fn describe(self) -> String {
        match self.limit {
            "entry count" => format!(
                "Suche nach {} Einträgen angehalten (Obergrenze); weitere Ordner wurden nicht durchsucht",
                crate::analytics::thousands(MAX_RECLAIM_ENTRIES)
            ),
            "path/name text" => format!(
                "Suche nach {} MiB Pfadtext angehalten (Obergrenze); weitere Ordner wurden nicht durchsucht",
                MAX_RECLAIM_TEXT_BYTES / (1024 * 1024)
            ),
            _ => format!(
                "Suche bei Ordnertiefe {MAX_RECLAIM_DEPTH} angehalten (Obergrenze); weitere Ordner wurden nicht durchsucht"
            ),
        }
    }
}

/// The German wording of a `ReclaimReport::scan_limit` text.
pub(super) fn describe_scan_limit(raw: &str) -> String {
    LIMITS
        .into_iter()
        .map(|limit| LimitExceeded { limit })
        .find(|known| known.to_string() == raw)
        .map_or_else(
            || format!("Suche vorzeitig angehalten ({raw})"),
            LimitExceeded::describe,
        )
}

/// `ReclaimBudget` for a parallel walk: the same limits, shared by atomic
/// counters and just as sticky (the first exceeded limit ends the walk).
pub(super) struct SharedBudget {
    entries: AtomicU64,
    text_bytes: AtomicU64,
    stopped: OnceLock<LimitExceeded>,
    max_entries: u64,
    max_text_bytes: u64,
    max_depth: u32,
}

impl Default for SharedBudget {
    fn default() -> Self {
        Self::with_limits(
            MAX_RECLAIM_ENTRIES,
            MAX_RECLAIM_TEXT_BYTES,
            MAX_RECLAIM_DEPTH,
        )
    }
}

impl SharedBudget {
    pub(super) fn with_limits(max_entries: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            entries: AtomicU64::new(0),
            text_bytes: AtomicU64::new(0),
            stopped: OnceLock::new(),
            max_entries,
            max_text_bytes,
            max_depth,
        }
    }

    pub(super) fn stopped(&self) -> bool {
        self.stopped.get().is_some()
    }

    pub(super) fn limit(&self) -> Option<LimitExceeded> {
        self.stopped.get().copied()
    }

    pub(super) fn claim(
        &self,
        inspected_text_bytes: usize,
        depth: u32,
    ) -> Result<(), LimitExceeded> {
        if let Some(limit) = self.stopped.get() {
            return Err(*limit);
        }
        let text = u64::try_from(inspected_text_bytes).unwrap_or(u64::MAX);
        let limit = if depth > self.max_depth {
            "depth"
        } else if !add_within(&self.entries, 1, self.max_entries) {
            "entry count"
        } else if !add_within(&self.text_bytes, text, self.max_text_bytes) {
            "path/name text"
        } else {
            return Ok(());
        };
        let first = self.stopped.get_or_init(|| LimitExceeded { limit });
        Err(*first)
    }
}

fn add_within(counter: &AtomicU64, amount: u64, maximum: u64) -> bool {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(amount).filter(|next| *next <= maximum)
        })
        .is_ok()
}

impl Default for ReclaimBudget {
    fn default() -> Self {
        Self::with_limits(
            MAX_RECLAIM_ENTRIES,
            MAX_RECLAIM_TEXT_BYTES,
            MAX_RECLAIM_DEPTH,
        )
    }
}

impl ReclaimBudget {
    fn with_limits(max_entries: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            entries: 0,
            text_bytes: 0,
            max_entries,
            max_text_bytes,
            max_depth,
            stopped: None,
        }
    }

    pub(super) fn stopped(&self) -> bool {
        self.stopped.is_some()
    }

    pub(super) fn claim(
        &mut self,
        inspected_text_bytes: usize,
        depth: u32,
    ) -> Result<(), LimitExceeded> {
        if let Some(limit) = self.stopped {
            return Err(limit);
        }
        let inspected_text_bytes = u64::try_from(inspected_text_bytes).unwrap_or(u64::MAX);
        let next_entries = self
            .entries
            .checked_add(1)
            .filter(|next| *next <= self.max_entries);
        let next_text = self
            .text_bytes
            .checked_add(inspected_text_bytes)
            .filter(|next| *next <= self.max_text_bytes);
        let failure = if depth > self.max_depth {
            Some(LimitExceeded { limit: "depth" })
        } else if next_entries.is_none() {
            Some(LimitExceeded {
                limit: "entry count",
            })
        } else if next_text.is_none() {
            Some(LimitExceeded {
                limit: "path/name text",
            })
        } else {
            None
        };
        if let Some(failure) = failure {
            self.stopped = Some(failure);
            return Err(failure);
        }
        self.entries = next_entries.expect("checked above");
        self.text_bytes = next_text.expect("checked above");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_stops_at_each_limit_without_consuming_failed_claims() {
        let mut entries = ReclaimBudget::with_limits(1, 100, 4);
        assert!(entries.claim(1, 1).is_ok());
        assert_eq!(
            entries.claim(1, 1),
            Err(LimitExceeded {
                limit: "entry count"
            })
        );

        let mut text = ReclaimBudget::with_limits(2, 3, 4);
        assert_eq!(
            text.claim(4, 1),
            Err(LimitExceeded {
                limit: "path/name text"
            })
        );

        let mut depth = ReclaimBudget::with_limits(2, 100, 1);
        assert_eq!(depth.claim(1, 2), Err(LimitExceeded { limit: "depth" }));
    }

    #[test]
    fn android_background_task_shared_budget_stops_like_the_walk_budget() {
        let entries = SharedBudget::with_limits(2, 100, 4);
        assert!(entries.claim(1, 1).is_ok());
        assert!(entries.claim(1, 1).is_ok());
        assert!(!entries.stopped());
        let stop = entries.claim(1, 1).unwrap_err();
        assert_eq!(
            stop,
            LimitExceeded {
                limit: "entry count"
            }
        );
        // Sticky: even a claim that would fit is refused afterwards.
        assert_eq!(entries.claim(0, 0), Err(stop));
        assert_eq!(entries.limit(), Some(stop));

        let text = SharedBudget::with_limits(10, 3, 4);
        assert_eq!(
            text.claim(4, 1),
            Err(LimitExceeded {
                limit: "path/name text"
            })
        );
        let depth = SharedBudget::with_limits(10, 100, 1);
        assert_eq!(depth.claim(1, 2), Err(LimitExceeded { limit: "depth" }));

        let parallel = SharedBudget::with_limits(1000, u64::MAX, 4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| while parallel.claim(1, 1).is_ok() {});
            }
        });
        assert_eq!(parallel.entries.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn android_background_task_limits_read_in_german() {
        for limit in LIMITS {
            let text = describe_scan_limit(&LimitExceeded { limit }.to_string());
            assert!(text.starts_with("Suche "), "{text}");
            assert!(!text.contains("bounded"), "{text}");
        }
        assert!(describe_scan_limit("other").contains("other"));
    }
}
