use super::*;

impl App {
    /// Leave a disconnected remote/network source without retaining paths that
    /// could later be mistaken for local filesystem entries.
    pub(in crate::app) fn clear_disconnected_source_view(&mut self) {
        self.cancel_analytics_scan();
        self.cancel_reclaim_scan();
        self.navigate_to_landing_page();
    }
}
