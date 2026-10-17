//! Desktop updates replace `se` only after every process running it has
//! exited: the updater helper writes `daemon.stop` = `stop` and waits at most
//! 30 seconds (`bin/smart_explorer_updater/process.rs`). A server held open by
//! an agent session must therefore end on that request. A request already
//! present at start is ignored (a crashed update can leave it behind); a new
//! or rewritten one ends the server. Version handoffs of `se update` are not
//! affected: they never stop other `se` processes. The rule and the poll
//! interval come from the shared helper `daemon::update_stop` (V8), which
//! every other long-running process of the GUI and CLI images uses as well.
use crate::daemon::{UpdateStopBaseline, UPDATE_STOP_POLL};

#[cfg(test)]
use crate::daemon::is_new_request;

pub(super) struct UpdateStop {
    baseline: UpdateStopBaseline,
}

impl UpdateStop {
    pub(super) fn arm() -> Self {
        Self {
            baseline: UpdateStopBaseline::arm(),
        }
    }

    pub(super) fn requested(&self) -> bool {
        self.baseline.requested()
    }

    /// Resolves once an update asks every `se` process to exit.
    pub(super) async fn wait(self) {
        loop {
            tokio::time::sleep(UPDATE_STOP_POLL).await;
            if self.requested() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_new_request;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn mcp_task_only_new_update_stop_requests_end_the_server() {
        let old = UNIX_EPOCH + Duration::from_secs(10);
        let new = UNIX_EPOCH + Duration::from_secs(20);
        assert!(!is_new_request(None, None));
        assert!(is_new_request(None, Some(new)));
        assert!(!is_new_request(Some(old), Some(old)));
        assert!(is_new_request(Some(old), Some(new)));
        assert!(!is_new_request(Some(old), None));
    }
}
