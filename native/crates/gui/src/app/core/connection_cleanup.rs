//! GUI half of a removed connection. The headless cleanup of favourites,
//! per-folder preferences, mounts and orphaned sync jobs lives in
//! `crate::connect` and is re-exported here under its previous names (the CLI
//! reaches it as `crate::app::cleanup_removed_endpoint_state`).
pub use crate::connect::{cleanup_removed_endpoint_state, CleanupReport, RemovedEndpointScope};

impl super::App {
    /// GUI half of a removal: clean the persisted stores, reload the in-memory
    /// favourites/preferences, and close every tab that browses the endpoint.
    pub(in crate::app) fn cleanup_after_removal(
        &mut self,
        scope: &RemovedEndpointScope,
    ) -> CleanupReport {
        let mut report = cleanup_removed_endpoint_state(scope);
        self.favorites = crate::connect::load_favorites();
        self.dir_sort = crate::connect::load_dir_sort();
        let tab_matches = |remote: Option<&crate::connect::RemoteState>, root_path: &str| {
            remote
                .and_then(|remote| remote.endpoint_prefix.as_deref())
                .is_some_and(|prefix| scope.matches_key(prefix))
                || (root_path.starts_with("//") && scope.matches_key(root_path))
        };
        let inactive: Vec<usize> = (0..self.tabs.len())
            .filter(|&index| index != self.active_tab)
            .filter(|&index| {
                tab_matches(
                    self.tabs[index].remote.as_ref(),
                    &self.tabs[index].root_path,
                )
            })
            .collect();
        for index in inactive.into_iter().rev() {
            self.close_tab(index);
            report.tabs_closed += 1;
        }
        if tab_matches(self.remote.as_ref(), &self.root_path) {
            self.clear_disconnected_source_view();
            report.tabs_closed += 1;
        }
        report
    }
}
