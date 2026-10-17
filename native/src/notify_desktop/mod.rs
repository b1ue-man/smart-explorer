//! Unattended desktop sync problems, delivered away from the scheduler, and
//! the notice of an access confirmation (PRM-M3, C5) beside its window.
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "os/android.rs"]
mod platform;
use crate::syncjobs::{ProblemKind, ProblemNotice};
use std::sync::OnceLock;
static SINK: OnceLock<Option<crossbeam_channel::Sender<ProblemNotice>>> = OnceLock::new();
pub(crate) fn notify(notice: &ProblemNotice) {
    deliver(
        notice.clone(),
        "Systembenachrichtigung nicht zugestellt; Sync-Problem bleibt im Jobstatus.",
    );
}

/// A device asks for access here; the consent window and `se share exec
/// approvals` stay the places to answer, this notice only points to them.
pub(crate) fn notify_access(title: &str, text: &str) {
    let notice = ProblemNotice {
        job_id: String::new(),
        job_name: String::new(),
        kind: ProblemKind::NeedsAction,
        title: title.to_string(),
        text: text.to_string(),
    };
    deliver(
        notice,
        "Systembenachrichtigung zur Zugriffsrückfrage nicht zugestellt; die Rückfrage bleibt offen.",
    );
}

fn deliver(notice: ProblemNotice, undelivered: &str) {
    let sink = SINK.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::bounded::<ProblemNotice>(32);
        match std::thread::Builder::new()
            .name("sync-notifications".into())
            .spawn(move || {
                for notice in receiver {
                    if let Err(error) = platform::notify(&notice) {
                        crate::daemon::log_worker(&format!("system notification failed: {error}"));
                    }
                }
            }) {
            Ok(_) => Some(sender),
            Err(_) => None,
        }
    });
    if sink
        .as_ref()
        .is_none_or(|sink| sink.try_send(notice).is_err())
    {
        crate::daemon::log_worker(undelivered);
    }
}
