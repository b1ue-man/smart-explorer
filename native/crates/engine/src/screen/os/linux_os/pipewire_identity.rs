//! A portal grant is a single stream in a single portal session, never a reusable node id.
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError, Weak};

/// Identity and terminal lifetime shared by capture and input.
pub struct StreamIdentity {
    pub node_id: u32,
    pub serial: Option<u64>,
    pub session: String,
    pub stream_id: Option<String>,
    /// Compositor coordinates, deliberately independent of PipeWire pixel
    /// dimensions: the size of the portal's answer when the source was
    /// chosen (`current_logical_size` follows later formats).
    pub logical_size: Option<(i32, i32)>,
    /// The first negotiated picture size and the logical size of the
    /// current format (review 2026-11-19, P-Portal).
    scaled: Mutex<(Option<(u32, u32)>, Option<(i32, i32)>)>,
    // 0: connecting; 1: connected; 2: terminal. A terminal grant cannot revive.
    state: AtomicU8,
    latest: Mutex<Option<Weak<super::LatestFrame>>>,
}

impl StreamIdentity {
    pub fn new(
        node_id: u32,
        serial: Option<u64>,
        session: String,
        stream_id: Option<String>,
        logical_size: Option<(i32, i32)>,
    ) -> Self {
        Self {
            node_id,
            serial,
            session,
            stream_id,
            logical_size,
            scaled: Mutex::new((None, None)),
            state: AtomicU8::new(0),
            latest: Mutex::new(None),
        }
    }

    /// The compositor coordinates of the stream now, for pointer input.
    pub fn current_logical_size(&self) -> Option<(i32, i32)> {
        let scaled = self.scaled.lock().unwrap_or_else(PoisonError::into_inner);
        scaled.1.or(self.logical_size)
    }

    /// A format of `width` × `height` pixels was negotiated (review
    /// 2026-11-19, P-Portal). The portal tells the logical size once, when
    /// the source is chosen; the first format is that moment's picture, so
    /// logical / picture size is the compositor's scale (1 where the stream
    /// carries logical pixels, 1 / scale where it carries device pixels).
    /// A new resolution at the same scale keeps that ratio, so the logical
    /// size follows the new picture size. A change of the scale alone at the
    /// same resolution changes no picture size and stays unseen [assumption:
    /// no PipeWire format tells the scale]; the portal answers it with a
    /// new source only.
    pub(super) fn on_format(&self, width: u32, height: u32) {
        // A source the portal named no size for keeps none: its picture size
        // is no compositor size ("the size may not be equivalent to a size
        // in a pixel coordinate space", ashpd 0.13 `Stream::size`, after the
        // portal's ScreenCast docs), so its pointer input is refused with a
        // reason (`portal_input::send_input`, Gemini 3/11).
        let Some((logical_width, logical_height)) = self.logical_size else {
            return;
        };
        let mut scaled = self.scaled.lock().unwrap_or_else(PoisonError::into_inner);
        let (first_width, first_height) = *scaled.0.get_or_insert((width, height));
        if first_width == 0 || first_height == 0 {
            return;
        }
        let follow = |size: u32, logical: i32, first: u32| {
            let value = (i64::from(size) * i64::from(logical) * 2 + i64::from(first))
                / (2 * i64::from(first));
            i32::try_from(value.max(1)).unwrap_or(i32::MAX)
        };
        scaled.1 = Some((
            follow(width, logical_width, first_width),
            follow(height, logical_height, first_height),
        ));
    }

    pub fn connected(&self) -> bool {
        self.state.load(Ordering::Acquire) == 1
    }

    pub fn expired(&self) -> bool {
        self.state.load(Ordering::Acquire) == 2
    }

    pub fn invalidate(&self) {
        self.state.store(2, Ordering::Release);
        if let Some(latest) = self
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
        {
            latest.fail("portal_source_expired: choose the source again".into());
        }
    }

    pub(super) fn attach(&self, latest: Weak<super::LatestFrame>) {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(latest);
        if self.expired() {
            self.invalidate();
        }
    }

    /// PAUSED is a connected, deliberately inactive stream, not a lost source.
    pub(super) fn on_state(&self, old: i32, new: i32) {
        if new < 0 || (new == 0 && old != 0) {
            self.invalidate();
        } else if new >= 2 {
            let _ = self
                .state
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    pub fn belongs_to(&self, session: &str) -> bool {
        self.connected() && self.session == session
    }

    pub(super) fn target(&self) -> (u32, Option<String>) {
        match self.serial {
            Some(serial) => (
                super::super::pipewire_abi::PW_ID_ANY,
                Some(serial.to_string()),
            ),
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
        let first = Arc::new(StreamIdentity::new(
            42,
            Some(u64::MAX),
            "/session/first".into(),
            Some("selected-window".into()),
            Some((960, 540)),
        ));
        let latest = Arc::new(super::super::LatestFrame::default());
        first.attach(Arc::downgrade(&latest));
        assert!(!first.belongs_to("/session/first"));
        first.on_state(1, 2);
        assert!(first.belongs_to("/session/first"));
        assert!(!first.belongs_to("/session/second"));
        assert_eq!(first.logical_size, Some((960, 540)));
        // Review 2026-11-19 (P-Portal): device pixels at scale 2, then a new
        // resolution at the same scale.
        assert_eq!(first.current_logical_size(), Some((960, 540)));
        first.on_format(1920, 1080);
        assert_eq!(first.current_logical_size(), Some((960, 540)));
        first.on_format(2560, 1440);
        assert_eq!(first.current_logical_size(), Some((1280, 720)));
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
