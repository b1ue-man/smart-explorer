//! Legacy and incompatible sessions close on a new stop marker. Managed
//! sessions retain stdio only for a matching identity-bound transaction and
//! an explicitly admitted hash/semantic contract. A completed matching marker
//! is consumed once; a fresh plain stop and orphaned prepared owner never
//! inherit that exception. The shared poll and close grace remain unchanged.
use crate::daemon::{UpdateStopBaseline, UPDATE_STOP_POLL};

#[cfg(test)]
use crate::daemon::is_new_request;

pub(super) struct UpdateStop {
    baseline: UpdateStopBaseline,
    marker: Option<crate::mcp_launcher::core::lifecycle::StopIdentity>,
}

impl UpdateStop {
    pub(super) fn arm() -> Self {
        Self {
            baseline: UpdateStopBaseline::arm(),
            marker: crate::mcp_launcher::stop_marker_identity(
                &crate::support_dirs::sync_data_dir().join("daemon.stop"),
            ),
        }
    }

    pub(super) fn requested(&self) -> bool {
        let current = crate::mcp_launcher::stop_marker_identity(
            &crate::support_dirs::sync_data_dir().join("daemon.stop"),
        );
        self.baseline.requested() || (current.is_some() && current != self.marker)
    }

    /// Resolves once an update asks every `se` process to exit.
    pub(super) async fn wait(
        mut self,
        session: std::sync::Arc<super::session::Session>,
        lifecycle: crate::mcp_launcher::LifecycleWatch,
    ) {
        loop {
            tokio::time::sleep(UPDATE_STOP_POLL).await;
            if lifecycle.parent_ended() {
                return;
            }
            if self.requested() {
                let negotiated = session
                    .hello
                    .lock()
                    .map(|h| h.as_ref().is_some_and(|r| r.accepted))
                    .unwrap_or(false);
                let local_marker = crate::support_dirs::sync_data_dir().join("daemon.stop");
                let current = crate::mcp_launcher::stop_marker_identity(&local_marker);
                let marker = if current.is_some() && current != self.marker {
                    local_marker.clone()
                } else {
                    crate::mcp_launcher::installation_stop_marker().unwrap_or(local_marker.clone())
                };
                let transaction = match crate::mcp_launcher::retained_update_transaction(&marker) {
                    Ok(transaction) => transaction,
                    Err(error) if error.starts_with("lock_busy:") => continue,
                    Err(error) => {
                        eprintln!("se mcp: cannot validate update stop: {error}");
                        None
                    }
                };
                if negotiated && transaction.is_some() {
                    let record = serde_json::json!({"transaction_id":transaction,"runtime":crate::mcp_launcher::current_runtime().map(|r|r.contract.runtime_sha256)});
                    if let Err(error) = std::fs::write(
                        session.directory.join("update-retained.v1.json"),
                        record.to_string(),
                    ) {
                        eprintln!("se mcp: cannot acknowledge retained transaction: {error}");
                        return;
                    }
                    self.baseline = UpdateStopBaseline::arm();
                    self.marker = crate::mcp_launcher::stop_marker_identity(&local_marker);
                    continue;
                }
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
