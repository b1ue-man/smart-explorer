use std::io;
use std::sync::Arc;

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Clone)]
pub(super) struct HandshakeAdmission {
    known: Arc<super::fair_admission::Pool<String>>,
    unknown: Arc<super::fair_admission::Pool<String>>,
}
impl HandshakeAdmission {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            known: super::fair_admission::Pool::new(capacity),
            unknown: super::fair_admission::Pool::new((capacity / 8).max(1)),
        }
    }
    pub(super) fn enqueue(
        &self,
        peer: String,
        known: bool,
    ) -> io::Result<super::fair_admission::Ticket<String>> {
        if peer.is_empty() || peer.len() > 128 {
            return Err(limit_reached());
        }
        let pool = if known { &self.known } else { &self.unknown };
        pool.enqueue(peer)
    }
}

#[derive(Default)]
struct Completion {
    done: AtomicBool,
    changed: Notify,
}
pub(super) struct HandshakeCompletion(Arc<Completion>);
impl HandshakeCompletion {
    pub(super) async fn wait(self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.0.done.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}

/// Application-handshake admission of one connection: a permit of the fair
/// per-device pool (`fair_admission.rs`). There is deliberately no fixed cap
/// per EndpointId: the pool is work-conserving, so one device may use idle
/// capacity, and fairness takes it back as soon as other devices wait.
/// Repeated failures of one EndpointId are penalized separately
/// (`auth_penalty.rs`).
pub(super) struct ApplicationHandshakePermit {
    _fair: super::fair_admission::Permit<String>,
    completion: Arc<Completion>,
}

impl ApplicationHandshakePermit {
    pub(super) fn admitted(
        permit: super::fair_admission::Permit<String>,
    ) -> (Self, HandshakeCompletion) {
        let completion = Arc::new(Completion::default());
        (
            Self {
                _fair: permit,
                completion: completion.clone(),
            },
            HandshakeCompletion(completion),
        )
    }
}
impl Drop for ApplicationHandshakePermit {
    fn drop(&mut self) {
        self.completion.done.store(true, Ordering::Release);
        self.completion.changed.notify_waiters();
    }
}

fn limit_reached() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "remote handshake admission limit reached",
    )
}
