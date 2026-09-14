//! Bounded *retention* for the size tree. The budget never stops a scan:
//! every file is still counted into its directory's size and the progress
//! totals; what it limits is how many individual nodes (and how much name
//! text) the tree keeps for the treemap. Beyond the limits, files collapse
//! into one aggregate node per directory and directories keep their exact
//! recursive size without retained children.
use super::outcome::Diagnostics;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_ANALYTICS_NODES: u64 = 6_000_000;
const MAX_ANALYTICS_TEXT_BYTES: u64 = 768 * 1024 * 1024;
/// Deeper trees are still counted, but not descended into by recursion; the
/// scan threads carry a stack sized for this depth.
pub(super) const MAX_ANALYTICS_DEPTH: u32 = 2048;
/// Files kept individually per directory (the largest ones); the rest of a
/// huge directory becomes one aggregate node with the exact remaining size.
pub(crate) const MAX_RETAINED_FILES_PER_DIRECTORY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Retention {
    /// The node may be kept in the tree.
    Keep,
    /// Count it, but fold it into its parent instead of keeping a node.
    Aggregate,
}

pub(super) struct AnalyticsBudget {
    nodes: AtomicU64,
    text_bytes: AtomicU64,
    aggregating: AtomicBool,
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
    pub(super) fn with_limits(max_nodes: u64, max_text_bytes: u64, max_depth: u32) -> Self {
        Self {
            nodes: AtomicU64::new(0),
            text_bytes: AtomicU64::new(0),
            aggregating: AtomicBool::new(false),
            max_nodes,
            max_text_bytes,
            max_depth,
        }
    }

    /// Whether the retained-node budget has been exhausted at least once.
    pub(super) fn aggregating(&self) -> bool {
        self.aggregating.load(Ordering::Relaxed)
    }

    pub(super) fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Whether recursion may descend to `depth`.
    pub(super) fn depth_allowed(&self, depth: u32) -> bool {
        depth <= self.max_depth
    }

    /// Reserve retention for one node. Aggregation is a one-way, one-time
    /// noted transition; counting continues regardless.
    pub(super) fn claim(
        &self,
        path: &Path,
        depth: u32,
        text_bytes: u64,
        diagnostics: &Diagnostics,
    ) -> Retention {
        if self.aggregating() {
            return Retention::Aggregate;
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
            return Retention::Keep;
        };
        if !self.aggregating.swap(true, Ordering::Relaxed) {
            diagnostics.note(format!(
                "Detailansicht ab {} zusammengefasst: das Limit fuer {limit} ist erreicht; Groessen und Zaehler bleiben vollstaendig",
                crate::analytics::os::display_path(path)
            ));
        }
        Retention::Aggregate
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
        // "Stops" retaining, never counting: exhaustion is a noted, one-way
        // switch to aggregation instead of an abort.
        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(1, 4, 2);
        assert_eq!(
            budget.claim(Path::new("root"), 0, 4, &diagnostics),
            Retention::Keep
        );
        assert_eq!(
            budget.claim(Path::new("second"), 1, 1, &diagnostics),
            Retention::Aggregate
        );
        assert!(budget.aggregating());
        assert_eq!(
            budget.claim(Path::new("third"), 1, 1, &diagnostics),
            Retention::Aggregate
        );
        let outcome = diagnostics.finish(
            super::super::SizeNode {
                name: "root".into(),
                size: 1,
                is_dir: true,
                children: Vec::new(),
            },
            false,
        );
        assert_eq!(outcome.status, super::super::ScanStatus::Complete);
        assert_eq!(outcome.notes.len(), 1);
        assert!(outcome.notes[0].contains("node count"));

        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(10, 3, 2);
        assert_eq!(
            budget.claim(Path::new("root"), 0, 4, &diagnostics),
            Retention::Aggregate
        );

        let diagnostics = Diagnostics::default();
        let budget = AnalyticsBudget::with_limits(10, 10, 1);
        assert_eq!(
            budget.claim(Path::new("deep"), 2, 1, &diagnostics),
            Retention::Aggregate
        );
        assert!(!budget.depth_allowed(2));
        assert!(budget.depth_allowed(1));
    }
}
