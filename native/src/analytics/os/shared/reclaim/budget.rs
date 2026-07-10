use std::fmt;

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
}
