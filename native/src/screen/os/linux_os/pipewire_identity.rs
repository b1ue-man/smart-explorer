//! A portal grant is a single stream in a single portal session, never a reusable node id.
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError, Weak};

/// Identity and terminal lifetime shared by capture and input.
pub(crate) struct StreamIdentity {
    pub node_id: u32,
    pub serial: Option<u64>,
    pub session: String,
    pub stream_id: Option<String>,
    /// Compositor coordinates, deliberately independent of PipeWire pixel dimensions.
    pub logical_size: Option<(i32, i32)>,
    // 0: connecting; 1: connected; 2: terminal. A terminal grant cannot revive.
    state: AtomicU8,
    latest: Mutex<Option<Weak<super::LatestFrame>>>,
}

impl StreamIdentity {
    pub(crate) fn new(
        node_id: u32,
        serial: Option<u64>,
        session: String,
        stream_id: Option<String>,
        logical_size: Option<(i32, i32)>,
    ) -> Self {
        Self { node_id, serial, session, stream_id, logical_size, state: AtomicU8::new(0), latest: Mutex::new(None) }
    }

    pub(crate) fn connected(&self) -> bool { self.state.load(Ordering::Acquire) == 1 }

    pub(crate) fn expired(&self) -> bool { self.state.load(Ordering::Acquire) == 2 }

    pub(crate) fn invalidate(&self) {
        self.state.store(2, Ordering::Release);
        if let Some(latest) = self.latest.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(Weak::upgrade) {
            latest.fail("portal_source_expired: choose the source again".into());
        }
    }

    pub(super) fn attach(&self, latest: Weak<super::LatestFrame>) {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(latest);
        if self.expired() { self.invalidate(); }
    }

    /// PAUSED is a connected, deliberately inactive stream, not a lost source.
    pub(super) fn on_state(&self, old: i32, new: i32) {
        if new < 0 || (new == 0 && old != 0) {
            self.invalidate();
        } else if new >= 2 {
            let _ = self.state.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    pub(crate) fn belongs_to(&self, session: &str) -> bool {
        self.connected() && self.session == session
    }

    pub(super) fn target(&self) -> (u32, Option<String>) {
        match self.serial {
            Some(serial) => (super::super::pipewire_abi::PW_ID_ANY, Some(serial.to_string())),
            None => (self.node_id, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn batches_task_p4d_portal_identity_expires_permanently_on_disconnect() {
        let first = Arc::new(StreamIdentity::new(42, Some(u64::MAX), "/session/first".into(),
            Some("selected-window".into()), Some((960, 540))));
        let latest = Arc::new(super::super::LatestFrame::default());
        first.attach(Arc::downgrade(&latest));
        assert!(!first.belongs_to("/session/first"));
        first.on_state(1, 2);
        assert!(first.belongs_to("/session/first"));
        assert!(!first.belongs_to("/session/second"));
        assert_eq!(first.logical_size, Some((960, 540)));
        assert_eq!(first.stream_id.as_deref(), Some("selected-window"));
        assert_eq!(first.target(), (u32::MAX, Some(u64::MAX.to_string())));
        first.on_state(2, 3);
        first.on_state(3, 2); // A deliberate capture pause preserves the grant.
        assert!(first.connected());
        first.on_state(2, 0);
        first.on_state(0, 3); // A reused node id must not revive it.
        assert!(first.expired());
        assert!(!first.belongs_to("/session/first"));
        assert!(matches!(latest.take(true), super::super::Taken::Failed(_)));
        let second = StreamIdentity::new(42, None, "/session/second".into(), None, None);
        second.on_state(1, 2);
        assert!(second.belongs_to("/session/second"));
        assert_eq!(second.target(), (42, None));
        first.invalidate();
        assert!(second.connected());
    }
}
