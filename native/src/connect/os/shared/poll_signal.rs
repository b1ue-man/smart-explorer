//! Owned polling subscriptions for provider cursors. Drop wakes the worker
//! immediately; a request already in flight keeps its provider deadline.
use crate::vfs::{ChangeNotice, ChangeSubscription};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub(crate) enum PollNotice {
    Quiet,
    Changed,
    Overflow,
}

pub(crate) fn poll_subscription(
    interval: Duration,
    tx: crossbeam_channel::Sender<ChangeNotice>,
    complete: bool,
    mut poll: impl FnMut(&AtomicBool) -> io::Result<PollNotice> + Send + 'static,
) -> io::Result<ChangeSubscription> {
    let (stop, stopped) = mpsc::channel::<()>();
    let canceled = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&canceled);
    std::thread::Builder::new()
        .name("provider-change-poll".into())
        .spawn(move || {
            let ready = if complete {
                ChangeNotice::Ready { generation: None }
            } else {
                ChangeNotice::ReadyPartial { generation: None }
            };
            if !send_notice(&tx, ready, &worker_cancel) {
                return;
            }
            let interval = interval.max(Duration::from_secs(1));
            loop {
                match stopped.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                // A subscription dropped during an HTTP call sends no late notice.
                let result = poll(&worker_cancel);
                if !matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                    break;
                }
                let notice = match result {
                    Ok(PollNotice::Quiet) => continue,
                    Ok(PollNotice::Changed) => ChangeNotice::Changed {
                        generation: None,
                        paths: Vec::new(),
                    },
                    Ok(PollNotice::Overflow) => ChangeNotice::Overflow,
                    Err(error) => {
                        send_notice(&tx, ChangeNotice::Ended(error.to_string()), &worker_cancel);
                        break;
                    }
                };
                if !send_notice(&tx, notice, &worker_cancel) {
                    break;
                }
            }
        })?;
    Ok(ChangeSubscription::new(PollStop {
        _stop: stop,
        canceled,
    }))
}

struct PollStop {
    _stop: mpsc::Sender<()>,
    canceled: Arc<AtomicBool>,
}
impl Drop for PollStop {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
    }
}

fn send_notice(
    tx: &crossbeam_channel::Sender<ChangeNotice>,
    mut notice: ChangeNotice,
    canceled: &AtomicBool,
) -> bool {
    loop {
        if canceled.load(Ordering::Acquire) {
            return false;
        }
        match tx.send_timeout(notice, Duration::from_millis(100)) {
            Ok(()) => return true,
            Err(crossbeam_channel::SendTimeoutError::Timeout(pending)) => notice = pending,
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return false,
        }
    }
}
