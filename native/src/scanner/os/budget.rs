use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const MAX_SCAN_ENTRIES: u64 = 1_000_000;
pub(super) const MAX_SCAN_TEXT_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_SCAN_DEPTH: u32 = 512;

pub(super) struct ScanBudget {
    entries: AtomicU64,
    text_bytes: AtomicU64,
    max_entries: u64,
    max_text_bytes: u64,
    max_depth: u32,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self::with_limits(MAX_SCAN_ENTRIES, MAX_SCAN_TEXT_BYTES, MAX_SCAN_DEPTH)
    }
}

impl ScanBudget {
    fn with_limits(max_entries: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            entries: AtomicU64::new(0),
            text_bytes: AtomicU64::new(0),
            max_entries,
            max_text_bytes,
            max_depth,
        }
    }

    pub(super) fn claim(&self, text_bytes: u64, depth: u32) -> Result<(), &'static str> {
        if depth > self.max_depth {
            return Err("depth");
        }
        claim_counter(&self.entries, 1, self.max_entries).map_err(|_| "entry count")?;
        claim_counter(&self.text_bytes, text_bytes, self.max_text_bytes)
            .map_err(|_| "retained path text")
    }
}

fn claim_counter(counter: &AtomicU64, amount: u64, maximum: u64) -> Result<(), ()> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(amount).filter(|next| *next <= maximum)
        })
        .map(|_| ())
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_budget_rejects_each_bound_without_allocating_a_tree() {
        let entries = ScanBudget::with_limits(2, 100, 4);
        assert!(entries.claim(1, 1).is_ok());
        assert!(entries.claim(1, 1).is_ok());
        assert_eq!(entries.claim(1, 1), Err("entry count"));

        let text = ScanBudget::with_limits(10, 5, 4);
        assert!(text.claim(5, 1).is_ok());
        assert_eq!(text.claim(1, 1), Err("retained path text"));

        let depth = ScanBudget::with_limits(10, 100, 2);
        assert_eq!(depth.claim(1, 3), Err("depth"));
    }
}
