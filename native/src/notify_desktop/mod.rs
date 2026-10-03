//! Unattended desktop sync problems, delivered away from the scheduler.
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "os/android.rs"]
mod platform;
use std::sync::OnceLock;
use crate::syncjobs::ProblemNotice;
static SINK: OnceLock<Option<crossbeam_channel::Sender<ProblemNotice>>> = OnceLock::new();
pub(crate) fn notify(notice: &ProblemNotice) {
    let sink = SINK.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::bounded::<ProblemNotice>(32);
        match std::thread::Builder::new().name("sync-notifications".into()).spawn(move || {
            for notice in receiver {
                if let Err(error) = platform::notify(&notice) {
                    crate::daemon::log_worker(&format!("system notification failed: {error}"));
                }
            }
        }) { Ok(_) => Some(sender), Err(_) => None }
    });
    if sink.as_ref().is_none_or(|sink| sink.try_send(notice.clone()).is_err()) {
        crate::daemon::log_worker("Systembenachrichtigung nicht zugestellt; Sync-Problem bleibt im Jobstatus.");
    }
}
