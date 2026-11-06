//! Rights changes for the device feeds of this host (A6 of plan
//! 2026-11-06-akku-rechte). A feed on a host without a sampleable source
//! (window families and listening services unsupported, no RDP host: the
//! Android device) sends its state once and then waits for a change of
//! rights instead of sampling every second. The share host reports every
//! change of its rights (`rights_changed`: a new authorization of the
//! worker, a grant that reached its end) and declares once that it does so
//! (`rights_reported`); before that declaration every feed keeps sampling
//! at its rate, so no change of rights can go unsent.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tokio::sync::watch;

static REPORTED: AtomicBool = AtomicBool::new(false);

fn signal() -> &'static watch::Sender<u64> {
    static SIGNAL: OnceLock<watch::Sender<u64>> = OnceLock::new();
    SIGNAL.get_or_init(|| watch::channel(0).0)
}

/// The share host reports every change of rights from now on.
pub fn rights_reported() {
    REPORTED.store(true, Ordering::Release);
}

/// Rights of this host changed: every waiting feed samples again.
pub fn rights_changed() {
    signal().send_modify(|count| *count = count.wrapping_add(1));
}

pub(super) fn reported() -> bool {
    REPORTED.load(Ordering::Acquire)
}

/// Subscribe before the sample it guards, so a change during the sample
/// is seen as unseen.
pub(super) fn subscribe() -> watch::Receiver<u64> {
    signal().subscribe()
}
