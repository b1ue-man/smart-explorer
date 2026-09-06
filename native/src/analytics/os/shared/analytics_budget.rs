use super::outcome::Diagnostics;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_ANALYTICS_NODES: u64 = 1_000_000;
const MAX_ANALYTICS_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ANALYTICS_DEPTH: u32 = 512;

pub(super) struct AnalyticsBudget {
    nodes: AtomicU64,
    text_bytes: AtomicU64,
    stopped: AtomicBool,
    max_nodes: u64,
    max_text_bytes: u64,
    max_depth: u32,
}

impl Default for AnalyticsBudget {
    fn default() -> Self {
        Self::with_limits(
            MAX_ANALYTICS_NODES,
            MAX_ANALYTICS_TEXT_BYTES,
            MAX_ANALYTICS_DEPTH,
        )
    }
}

impl AnalyticsBudget {
    fn with_limits(max_nodes: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            nodes: AtomicU64::new(0),
            text_bytes: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            max_nodes,
            max_text_bytes,
            max_depth,
        }
    }

    pub(super) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    pub(super) fn claim(
        &self,
        path: &Path,
        depth: u32,
        text_bytes: u64,
        diagnostics: &Diagnostics,
    ) -> bool {
        if self.stopped() {
            return false;
        }
        let failed = if depth > self.max_depth {
            Some("depth")
        } else if !claim_counter(&self.nodes, 1, self.max_nodes) {
            Some("node count")
        } else if !claim_counter(&self.text_bytes, text_bytes, self.max_text_bytes) {
            Some("retained name text")
        } else {
            None
        };
        let Some(limit) = failed else {
            return true;
        };
        if !self.stopped.swap(true, Ordering::Relaxed) {
            diagnostics.record(
                path.to_string_lossy().into_owned(),
                format!("analytics scan stopped at its bounded {limit} limit"),
                false,
            );
        }
        false
    }
}

fn claim_counter(counter: &AtomicU64, amount: u64, maximum: u64) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(amount).filter(|next| *next <= maximum)
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_access_task_existing_budget_stops_honestly() {
        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(1, 4, 2);
        assert!(budget.claim(Path::new("root"), 0, 4, &diagnostics));
        assert!(!budget.claim(Path::new("second"), 1, 1, &diagnostics));
        assert!(budget.stopped());

        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(10, 3, 2);
        assert!(!budget.claim(Path::new("root"), 0, 4, &diagnostics));

        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(10, 10, 1);
        assert!(!budget.claim(Path::new("deep"), 2, 1, &diagnostics));
    }
}
