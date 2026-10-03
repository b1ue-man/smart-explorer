//! Where problem notices of unattended runs go (RV1, contract V4). The
//! embedding host installs a notifier (Android: an event for the
//! "Sync-Probleme" channel); without one the desktop worker shows a system
//! notification itself. Which problem is due when is decided by
//! `syncjobs::take_problem_notices`, throttled per job and problem.

use std::sync::{Mutex, PoisonError};

use crate::syncjobs::ProblemNotice;

static NOTIFIER: Mutex<Option<fn(&ProblemNotice)>> = Mutex::new(None);

/// Installs the host's notifier; the latest call wins. It is called on the
/// worker thread and must return quickly.
pub fn set_problem_notifier(notifier: fn(&ProblemNotice)) {
    *NOTIFIER.lock().unwrap_or_else(PoisonError::into_inner) = Some(notifier);
}

pub(super) fn notify(jobs: &[crate::syncjobs::SyncJob], now: i64) {
    let notifier = *NOTIFIER.lock().unwrap_or_else(PoisonError::into_inner);
    for notice in crate::syncjobs::take_problem_notices(jobs, now) {
        match notifier { Some(notifier) => notifier(&notice), None => crate::notify_desktop::notify(&notice) }
    }
}
