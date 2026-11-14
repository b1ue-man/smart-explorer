//! Resolve and subscribe away from the scheduler and local event receiver.
use crate::bisync::PairSide;
use crate::vfs::{ChangeNotice, ChangeSignalMode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Start of an `Ended` text after which this side only polls (no change
/// signal, or a Share host that cannot watch the path: `watch_wire.rs`
/// `POLL_ONLY` holds the same words).
const POLL_ONLY: &str = "Keine Änderungsereignisse; Abfrage aktiv";

/// Whether a notice ends the subscription for good: subscribing again would
/// fail the same way, so the job keeps polling (M14).
fn polls_only(notice: &ChangeNotice) -> bool {
    matches!(notice, ChangeNotice::Ended(reason) if reason.starts_with(POLL_ONLY))
}

pub(super) struct Message {
    pub(super) side: PairSide,
    pub(super) event: ChangeNotice,
}

pub(super) fn start(
    endpoint: String,
    side: PairSide,
    interval: u64,
    stop: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
    sink: crossbeam_channel::Sender<Message>,
) {
    let error_sink = sink.clone();
    if let Err(error) = std::thread::Builder::new().name("sync-remote-watch".into()).spawn(move || {
        let poll = Duration::from_secs(if interval == 0 { 300 } else { interval });
        let mut retry = poll;
        while !stop.load(Ordering::Acquire) {
            let (backend, root) = match crate::connect::resolve_endpoint(&endpoint) {
                Ok(value) => value,
                Err(error) => {
                    let needs_user = super::job_triggers::connect_failure(&error).needs_user();
                    deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended(error) });
                    if needs_user { return; }
                    if wait(&stop, retry) { return; }
                    retry = retry.saturating_mul(2).min(Duration::from_secs(6 * 3600));
                    continue;
                }
            };
            let mode = crate::vfs::change_signal_mode(&*backend, &root).ok().flatten();
            if interval == 0 && mode == Some(ChangeSignalMode::Poll) {
                deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended("Änderungsabfrage ist ausgeschaltet".into()) });
                return;
            }
            let (sender, receiver) = crossbeam_channel::bounded(4096);
            let subscription = match crate::vfs::change_signal(&*backend, &root, poll, sender) {
                Ok(Some(subscription)) => subscription,
                Ok(None) => {
                    deliver(&sink, &overflow, Message { side,
                        event: ChangeNotice::Ended(POLL_ONLY.into()) });
                    return;
                }
                Err(error) => {
                    let needs_user = super::job_triggers::connect_failure(&error.to_string()).needs_user();
                    deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended(error.to_string()) });
                    if needs_user { return; }
                    if wait(&stop, retry) { return; }
                    retry = retry.saturating_mul(2).min(Duration::from_secs(6 * 3600));
                    continue;
                }
            };
            retry = poll;
            while !stop.load(Ordering::Acquire) {
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(event) => {
                        let needs_user = matches!(&event, ChangeNotice::Ended(error) if super::job_triggers::connect_failure(error).needs_user());
                        let ended = matches!(event, ChangeNotice::Ended(_));
                        let poll_only = polls_only(&event);
                        deliver(&sink, &overflow, Message { side, event });
                        if needs_user || poll_only { return; }
                        if ended { break; }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        deliver(&sink, &overflow, Message { side,
                            event: ChangeNotice::Ended("Änderungsabonnement beendet".into()) });
                        break;
                    }
                }
            }
            drop(subscription);
            if wait(&stop, retry) { return; }
        }
    }) {
        let _ = error_sink.try_send(Message { side, event: ChangeNotice::Ended(error.to_string()) });
    }
}

fn deliver(sink: &crossbeam_channel::Sender<Message>, overflow: &AtomicBool, message: Message) {
    if sink.try_send(message).is_err() {
        overflow.store(true, Ordering::Release);
    }
}

fn wait(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = std::time::Instant::now().checked_add(duration);
    while !stop.load(Ordering::Acquire) && deadline.is_some_and(|at| std::time::Instant::now() < at)
    {
        std::thread::sleep(Duration::from_secs(1));
    }
    stop.load(Ordering::Acquire)
}

#[cfg(test)]
mod windows_watch_task_tests {
    use super::*;

    #[test]
    fn windows_watch_task_unsupported_host_switches_the_peer_to_polling() {
        // The text `FsWatchEvent::into_notice` makes of an unsupported watch.
        let unsupported = "Keine Änderungsereignisse; Abfrage aktiv: Unsupported";
        assert!(polls_only(&ChangeNotice::Ended(unsupported.into())));
        assert!(polls_only(&ChangeNotice::Ended(POLL_ONLY.into())));
        // Transient ends still subscribe again after one poll interval.
        for reason in ["RootMissing", "AccessDenied", "Änderungsabonnement beendet"] {
            assert!(!polls_only(&ChangeNotice::Ended(reason.into())));
        }
        assert!(!polls_only(&ChangeNotice::Overflow));
    }
}
