//! Desktop updates replace `se` only after every process running it has
//! exited: the updater helper writes `daemon.stop` = `stop` and waits at most
//! 30 seconds (`bin/smart_explorer_updater/process.rs`). A server held open by
//! an agent session must therefore end on that request. A request already
//! present at start is ignored (a crashed update can leave it behind); a new
//! or rewritten one ends the server. Version handoffs of `se update` are not
//! affected: they never stop other `se` processes.
use std::time::{Duration, SystemTime};

/// Poll interval, well inside the helper's 30-second wait so jobs still get
/// their cancellation path.
const POLL: Duration = Duration::from_secs(1);

pub(super) struct UpdateStop {
    baseline: Option<SystemTime>,
}

impl UpdateStop {
    pub(super) fn arm() -> Self {
        Self {
            baseline: crate::daemon::update_stop_marker(),
        }
    }

    pub(super) fn requested(&self) -> bool {
        is_new_request(self.baseline, crate::daemon::update_stop_marker())
    }

    /// Resolves once an update asks every `se` process to exit.
    pub(super) async fn wait(self) {
        loop {
            tokio::time::sleep(POLL).await;
            if self.requested() {
                return;
            }
        }
    }
}

fn is_new_request(baseline: Option<SystemTime>, current: Option<SystemTime>) -> bool {
    match (baseline, current) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(before), Some(now)) => before != now,
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
