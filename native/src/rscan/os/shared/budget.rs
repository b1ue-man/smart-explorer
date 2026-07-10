use std::fmt;

pub(super) const MAX_SCAN_ENTRIES: u64 = 1_000_000;
pub(super) const MAX_SCAN_TEXT_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_SCAN_DEPTH: u32 = 512;

/// One shared, deterministic budget for remote walks and server-side search
/// results. The counters are updated by the driver thread after backend calls
/// finish, so serial and parallel listing paths have identical ordering and
/// limit behavior without atomics.
pub(super) struct ScanBudget {
    entries: u64,
    text_bytes: u64,
    max_entries: u64,
    max_text_bytes: u64,
    max_depth: u32,
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

impl Default for ScanBudget {
    fn default() -> Self {
        Self::with_limits(MAX_SCAN_ENTRIES, MAX_SCAN_TEXT_BYTES, MAX_SCAN_DEPTH)
    }
}

impl ScanBudget {
    fn with_limits(max_entries: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            entries: 0,
            text_bytes: 0,
            max_entries,
            max_text_bytes,
            max_depth,
        }
    }

    pub(super) fn claim(
        &mut self,
        retained_text_bytes: usize,
        depth: u32,
    ) -> Result<(), LimitExceeded> {
        if depth > self.max_depth {
            return Err(LimitExceeded { limit: "depth" });
        }
        let next_entries = self
            .entries
            .checked_add(1)
            .filter(|next| *next <= self.max_entries);
        let retained_text_bytes = u64::try_from(retained_text_bytes).unwrap_or(u64::MAX);
        let next_text = self
            .text_bytes
            .checked_add(retained_text_bytes)
            .filter(|next| *next <= self.max_text_bytes);
        match (next_entries, next_text) {
            (Some(entries), Some(text_bytes)) => {
                self.entries = entries;
                self.text_bytes = text_bytes;
                Ok(())
            }
            (None, _) => Err(LimitExceeded {
                limit: "entry count",
            }),
            (_, None) => Err(LimitExceeded {
                limit: "retained path/name text",
            }),
        }
    }

    pub(super) fn preflight_entries(&self, additional: usize) -> Result<(), LimitExceeded> {
        let additional = u64::try_from(additional).unwrap_or(u64::MAX);
        if self
            .entries
            .checked_add(additional)
            .is_some_and(|total| total <= self.max_entries)
        {
            Ok(())
        } else {
            Err(LimitExceeded {
                limit: "entry count",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_rejects_each_boundary_without_allocating_a_large_tree() {
        let mut entries = ScanBudget::with_limits(2, 100, 4);
        assert!(entries.claim(1, 1).is_ok());
        assert!(entries.claim(1, 1).is_ok());
        assert_eq!(
            entries.claim(1, 1),
            Err(LimitExceeded {
                limit: "entry count"
            })
        );

        let mut text = ScanBudget::with_limits(10, 5, 4);
        assert!(text.claim(5, 1).is_ok());
        assert_eq!(
            text.claim(1, 1),
            Err(LimitExceeded {
                limit: "retained path/name text"
            })
        );

        let mut depth = ScanBudget::with_limits(10, 100, 2);
        assert_eq!(depth.claim(1, 3), Err(LimitExceeded { limit: "depth" }));
    }

    #[test]
    fn failed_text_claim_does_not_consume_an_entry_slot() {
        let mut budget = ScanBudget::with_limits(1, 1, 1);
        assert!(budget.claim(2, 1).is_err());
        assert!(budget.claim(1, 1).is_ok());
    }

    #[test]
    fn listing_preflight_accounts_for_entries_already_claimed() {
        let mut budget = ScanBudget::with_limits(2, 100, 1);
        assert!(budget.claim(1, 1).is_ok());
        assert_eq!(
            budget.preflight_entries(2),
            Err(LimitExceeded {
                limit: "entry count"
            })
        );
    }
}
