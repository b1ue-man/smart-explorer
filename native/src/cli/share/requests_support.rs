pub(super) struct WorkerRefresh {
    pub(super) state: &'static str,
    pub(super) error: Option<String>,
}

impl WorkerRefresh {
    pub(super) fn value(&self) -> serde_json::Value {
        serde_json::json!({"state": self.state, "error": self.error})
    }
}

pub(super) fn worker_refresh() -> WorkerRefresh {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => WorkerRefresh {
            state: "refreshed",
            error: None,
        },
        Ok(false) => WorkerRefresh {
            state: "inactive",
            error: Some("Share server is not configured or Auto-Connect is off".to_string()),
        },
        Err(error) => WorkerRefresh {
            state: "unavailable",
            error: Some(error),
        },
    }
}
