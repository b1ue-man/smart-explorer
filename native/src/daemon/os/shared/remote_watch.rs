//! Resolve and subscribe away from the scheduler and local event receiver.
use crate::bisync::PairSide;
use crate::vfs::{ChangeNotice, ChangeSignalMode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct Message {
    pub(super) side: PairSide,
    pub(super) event: ChangeNotice,
    pub(super) mode: Option<ChangeSignalMode>,
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
                    deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended(error), mode: None });
                    if needs_user { return; }
                    if wait(&stop, retry) { return; }
                    retry = retry.saturating_mul(2).min(Duration::from_secs(6 * 3600));
                    continue;
                }
            };
            let mode = crate::vfs::change_signal_mode(&*backend, &root).ok().flatten();
            if interval == 0 && mode == Some(ChangeSignalMode::Poll) {
                deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended("Änderungsabfrage ist ausgeschaltet".into()), mode: None });
                return;
            }
            let (sender, receiver) = crossbeam_channel::bounded(4096);
            let subscription = match crate::vfs::change_signal(&*backend, &root, poll, sender) {
                Ok(Some(subscription)) => subscription,
                Ok(None) => {
                    deliver(&sink, &overflow, Message { side,
                        event: ChangeNotice::Ended("Keine Änderungsereignisse; Abfrage aktiv".into()), mode: None });
                    return;
                }
                Err(error) => {
                    let needs_user = super::job_triggers::connect_failure(&error.to_string()).needs_user();
                    deliver(&sink, &overflow, Message { side, event: ChangeNotice::Ended(error.to_string()), mode: None });
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
                        deliver(&sink, &overflow, Message { side, event, mode });
                        if needs_user { return; }
                        if ended { break; }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        deliver(&sink, &overflow, Message { side,
                            event: ChangeNotice::Ended("Änderungsabonnement beendet".into()), mode: None });
                        break;
                    }
                }
            }
            drop(subscription);
            if wait(&stop, retry) { return; }
        }
    }) {
        let _ = error_sink.try_send(Message { side, event: ChangeNotice::Ended(error.to_string()), mode: None });
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
