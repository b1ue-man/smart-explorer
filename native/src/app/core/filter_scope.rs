//! Filter changes against a (possibly pruned) recursive listing.
//!
//! A recursive scan started while a filter is active keeps only matching
//! entries and the directories needed to place them (`scan_retention`
//! remembers that filter). A later filter change may refilter those retained
//! entries only when the new filter is provably at least as narrow; otherwise
//! the scan restarts with the new filter. A listing the bounded budget cut
//! short (`scan_truncated`) restarts as soon as a different, pruning filter
//! could let the rescan finish.
use super::prelude::*;
use super::*;
use crate::filter::{filter_prunes, scan_restart_needed, FilterRetention};
use crate::scanner::RetentionHandle;

/// What the next scan starts with: the filter it prunes with (if any), the
/// walker's retention handle and the depth ceiling the filter implies.
pub(in crate::app) struct ScanScope {
    pub(in crate::app) pruned_with: Option<FilterDef>,
    pub(in crate::app) retention: Option<RetentionHandle>,
    pub(in crate::app) max_depth: Option<u32>,
}

impl App {
    /// A recursive scan prunes with the active filter; a flat listing never
    /// prunes because its single directory stays cheap to refilter. Call after
    /// `root_path` is set: glob filters match relative to it.
    pub(in crate::app) fn scan_scope(&self) -> ScanScope {
        if !self.recursive {
            return ScanScope {
                pruned_with: None,
                retention: None,
                max_depth: Some(1),
            };
        }
        if !filter_prunes(&self.filter) {
            return ScanScope {
                pruned_with: None,
                retention: None,
                max_depth: None,
            };
        }
        let retention = FilterRetention::new(self.filter.clone(), self.root_prefix());
        let max_depth = retention.max_depth();
        ScanScope {
            pruned_with: Some(self.filter.clone()),
            retention: Some(Arc::new(retention)),
            max_depth,
        }
    }

    /// The active filter changed: refilter the retained entries when they are
    /// complete for the new filter, otherwise start a new (pruned) scan.
    pub(in crate::app) fn filter_changed(&mut self) {
        let restart = self.recursive
            && !self.root_path.is_empty()
            && scan_restart_needed(
                self.scan_retention.as_ref(),
                self.scan_truncated,
                &self.filter,
            );
        if restart {
            self.rescan();
        } else {
            self.recompute_view();
        }
    }

    /// A filter narrowed during a running scan must apply to the unvisited
    /// part as well. Retry once with that filter when the old scan truncates.
    pub(in crate::app) fn note_scan_finished(&mut self, truncated: bool) {
        self.scan_truncated = truncated;
        if truncated
            && self.recursive
            && !self.scan_was_canceled
            && scan_restart_needed(self.scan_retention.as_ref(), true, &self.filter)
        {
            self.rescan();
            self.notice = Some((
                "Scan-Limit erreicht – die Suche wird mit dem inzwischen geänderten Filter fortgesetzt."
                    .to_string(),
                Instant::now(),
            ));
        } else if truncated {
            self.notice = Some((
                "⚠ Teilergebnis: Scan-Limit erreicht. Geladene Treffer bleiben erhalten; ein engerer Filter durchsucht erneut."
                    .to_string(),
                Instant::now(),
            ));
        }
    }
}
