//! Agent change subscriptions own their bounded delivery and cancellation guard.
use std::io;
use std::sync::Arc;

use super::{
    query, stream, unexpected, vfs, AgentBackend, AtomicBool, BackendExtensions,
    ChangeNotice, ChangeSignalMode, ChangeSubscription, Duration, Frame, Ordering,
    Sender, VfsResult, CANCEL_POLL,
};
use super::super::ext_wire::change_from_wire;

fn send_notice(
    tx: &Sender<ChangeNotice>,
    mut notice: ChangeNotice,
    cancel: &AtomicBool,
) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "change subscription canceled",
            ));
        }
        match tx.send_timeout(notice, CANCEL_POLL) {
            Ok(()) => return Ok(()),
            Err(crossbeam_channel::SendTimeoutError::Timeout(pending)) => notice = pending,
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "receiver gone"))
            }
        }
    }
}

impl AgentBackend {
    pub(super) fn agent_change_signal_mode(&self, root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        if !self.serves_peer() {
            if self.features().service {
                return Ok(None);
            }
            return vfs::change_signal_mode(&*self.inner, root);
        }
        Ok(match self.query(query::CHANGE_SIGNAL, root)? {
            1 => Some(ChangeSignalMode::Push),
            2 => Some(ChangeSignalMode::Poll),
            _ => None,
        })
    }

    pub(super) fn agent_change_signal(
        &self,
        root: &str,
        poll_interval: Duration,
        tx: Sender<ChangeNotice>,
    ) -> VfsResult<Option<ChangeSubscription>> {
        if !self.serves_peer() {
            if self.features().service {
                return Ok(None);
            }
            return vfs::change_signal(&*self.inner, root, poll_interval, tx);
        }
        if self.change_signal_mode(root)?.is_none() {
            return Ok(None);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let pool = self.pool.clone();
        let request = Frame::Watch {
            root: root.to_string(),
            poll_ms: u64::try_from(poll_interval.as_millis()).unwrap_or(u64::MAX),
        };
        let watching = stop.clone();
        let thread = std::thread::Builder::new()
            .name("agent-change-signal".into())
            .spawn(move || {
                let notices = tx.clone();
                let result = stream(&pool, request, &watching, |frame| match frame {
                    Frame::Change(change) => {
                        send_notice(&notices, change_from_wire(change), &watching)
                    }
                    other => Err(unexpected("change signal", &other)),
                });
                if !watching.load(Ordering::Relaxed) {
                    let reason = match result {
                        Ok(()) => "Änderungs-Abo beendet".to_string(),
                        Err(error) => error.to_string(),
                    };
                    let _ = send_notice(&tx, ChangeNotice::Ended(reason), &watching);
                }
            })?;
        Ok(Some(ChangeSubscription::new(WatchGuard {
            stop,
            thread: Some(thread),
        })))
    }
}

/// Ends a change subscription when dropped: the stream cancels and its
/// thread ends within one poll period.
struct WatchGuard {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
