//! What `se exec` reports on stderr about a Share start: the wait for the
//! peer's user, the account and, with `--verbose`, the shell that ran.
/// Which status of a Share start goes to stderr.
#[derive(Clone, Copy)]
pub(super) struct StatusReport {
    /// The account line (`{device}: as {account}`) and the shell that ran.
    pub(super) verbose: bool,
    /// JSON lines instead of text.
    pub(super) json: bool,
}

impl StatusReport {
    /// The start waits for the peer's user (PRM-M3): once per request.
    pub(super) fn waiting(
        self,
        device: &str,
        approval: &crate::share::exec_access_view::ExecApprovalPending,
    ) {
        let surface = approval.surface();
        if self.json {
            let event = crate::share::exec_client_messages::approval_pending_event(device, surface);
            eprintln!("{event}");
        } else {
            let line = crate::share::exec_client_messages::approval_status_line(device, surface);
            eprintln!("{line}");
        }
    }

    /// The shell that ran the command (`exec_peer::result_shell`).
    pub(super) fn shell(self, shell: Option<&str>) {
        if !self.verbose {
            return;
        }
        let shell = shell.unwrap_or("unknown");
        if self.json {
            eprintln!("{}", serde_json::json!({"event": "shell", "shell": shell}));
        } else {
            eprintln!("se: ran in {shell}");
        }
    }

    /// The account line once the peer authorized the start.
    pub(super) fn account(self, exec_id: &crate::share::ExecId, label: &str, user_label: &str) {
        if !self.verbose {
            return;
        }
        let notice = crate::daemon::exec_notice(exec_id).ok().flatten();
        let device = peer_name(notice.as_ref(), label);
        let access = notice.as_ref().and_then(|notice| notice.access.as_ref());
        let line = crate::share::exec_client_messages::account_line(&device, access, user_label);
        if self.json {
            let event = serde_json::json!({"event": "account", "device": device, "line": line});
            eprintln!("{event}");
        } else {
            eprintln!("{line}");
        }
    }
}

/// The peer as the worker names it, else as the user selected it.
pub(super) fn peer_name(notice: Option<&crate::daemon::ExecNoticeView>, label: &str) -> String {
    notice
        .map(|notice| notice.device.clone())
        .filter(|device| !device.trim().is_empty())
        .unwrap_or_else(|| label.to_string())
}
