//! The existing channel lock owns epochs, pictures, control, grants and retirement.
use super::*;

impl Channel {
    pub(super) fn new(id: u64, start: HostStart) -> Self {
        Self {
            id,
            start,
            state: Mutex::new((Phase::Consent, Queues::default())),
            changed: Condvar::new(),
            capture: super::super::android_capture_admission::CaptureSession::default(),
        }
    }

    pub(crate) fn phase(&self) -> Phase {
        lock(&self.state).0.clone()
    }

    /// Queues `message` for the app.
    pub(crate) fn send(&self, message: Value) {
        let mut state = lock(&self.state);
        if message.get("op").and_then(Value::as_str) == Some("configure") {
            // Retire old queued storage atomically with its epoch. A producer
            // cannot enqueue an old-epoch owner between clear and rebinding.
            state.1.pictures.clear();
            state.1.active_epoch = message
                .get("epoch")
                .and_then(Value::as_u64)
                .and_then(|epoch| u32::try_from(epoch).ok());
            state.1.encoder_failure = None;
        }
        if state.1.control.len() >= MAX_CONTROL {
            state.1.control.pop_front();
        }
        state.1.control.push_back(message.to_string());
        self.changed.notify_all();
    }

    /// Drops every queued picture (a new encoder epoch begins).
    pub(crate) fn clear_pictures(&self) {
        lock(&self.state).1.pictures.clear();
    }

    /// The next picture that `accept` takes, waiting at most `timeout`;
    /// pictures it refuses are dropped. `Err` with the reason once ended.
    pub(crate) fn next_picture(
        &self,
        timeout: Duration,
        accept: impl Fn(&Picture) -> bool,
    ) -> Result<Option<Picture>, String> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if let Phase::Ended(reason) = &state.0 {
                return Err(reason.clone());
            }
            if let Some(reason) = state.1.encoder_failure.take() {
                return Err(format!("encoder_failed: {reason}"));
            }
            while let Some(picture) = state.1.pictures.pop_front() {
                if accept(&picture) {
                    return Ok(Some(picture));
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits at most `timeout` for a phase other than `Consent`.
    pub(crate) fn wait_started(&self, timeout: Duration) -> Phase {
        let state = lock(&self.state);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.0 == Phase::Consent)
            .unwrap_or_else(PoisonError::into_inner);
        state.0.clone()
    }

    pub(super) fn set_phase(&self, phase: Phase) -> bool {
        let mut state = lock(&self.state);
        if matches!(state.0, Phase::Ended(_)) {
            return false;
        }
        if matches!(phase, Phase::Ended(_)) {
            state.1.pictures.clear();
            self.capture.retire();
        }
        state.0 = phase;
        drop(state);
        self.changed.notify_all();
        true
    }

    pub(super) fn push(&self, generation: u64, data: &[u8], flags: u32) -> i32 {
        let mut state = lock(&self.state);
        if matches!(state.0, Phase::Ended(_)) {
            return PUSH_UNKNOWN;
        }
        let Ok(grant) = self.capture.grant(generation) else {
            return PUSH_INVALID;
        };
        let epoch = flags >> EPOCH_SHIFT;
        if flags & (FLAG_STILL | FLAG_CAPTURED_VIDEO | FLAG_KEYFRAME) != 0
            && epoch != 0
            && state.1.active_epoch.is_some_and(|active| active != epoch)
        {
            return PUSH_OK;
        }
        if flags & 0x8 != 0 {
            if data.len() > 4096 {
                return PUSH_INVALID;
            }
            if state.1.active_epoch.is_some_and(|active| active != epoch) {
                return PUSH_OK;
            }
            state.1.encoder_failure = Some(String::from_utf8_lossy(data).into_owned());
            state.1.pictures.clear();
            self.changed.notify_all();
            return PUSH_OK;
        }
        if flags & 0x20 != 0 {
            if data.len() != 9 || data[0] > 1 {
                return PUSH_INVALID;
            }
            state.1.pictures.clear();
            state.1.encoder_failure = None;
            state.1.waiting_keyframe = true;
            let Ok(data) = self.capture.activity_ack(data) else {
                return PUSH_INVALID;
            };
            let picture = Picture { data, flags };
            state.1.pictures.push_back(picture);
            self.changed.notify_all();
            return PUSH_OK;
        }
        let queued_bytes: usize = state.1.pictures.iter().map(|p| p.data.len()).sum();
        let raw = flags & (FLAG_STILL | FLAG_CAPTURED_VIDEO) != 0;
        let overflow = if raw {
            !state.1.pictures.is_empty()
        } else {
            queued_bytes.saturating_add(data.len()) > 16 * 1024 * 1024
        };
        if state.1.pictures.len() >= MAX_QUEUED || overflow {
            state.1.pictures.clear();
            state.1.waiting_keyframe = true;
            if state.1.control.len() < MAX_CONTROL {
                state
                    .1
                    .control
                    .push_back(json!({"op":"keyframe"}).to_string());
            }
        }
        if flags & FLAG_STILL == 0 {
            if flags & FLAG_KEYFRAME != 0 {
                state.1.waiting_keyframe = false;
            } else if state.1.waiting_keyframe {
                return PUSH_OK;
            }
        }
        // Queue eviction and grant validation happen under this lock, BEFORE
        // the replacement copy. Producer + queued + consumer remain three J.
        let Ok(data) = AdmittedBytes::copy(&grant, CaptureOwner::Transport, data) else {
            return PUSH_INVALID;
        };
        let picture = Picture { data, flags };
        state.1.pictures.push_back(picture);
        self.changed.notify_all();
        PUSH_OK
    }

    pub(crate) fn begin_capture(
        &self,
        size: crate::screen::core::pixel_convert::FrameSize,
        kind: u32,
    ) -> std::io::Result<u64> {
        let state = lock(&self.state);
        if matches!(state.0, Phase::Ended(_)) {
            return Err(std::io::Error::other("Android capture session ended"));
        }
        self.capture.begin(size, kind)
    }

    pub(super) fn take_control(&self, timeout: Duration) -> Option<String> {
        let state = lock(&self.state);
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.1.control.is_empty() && !matches!(state.0, Phase::Ended(_))
            })
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(message) = state.1.control.pop_front() {
            return Some(message);
        }
        match &state.0 {
            Phase::Ended(reason) => Some(json!({"op": "stop", "reason": reason}).to_string()),
            _ => None,
        }
    }
}
