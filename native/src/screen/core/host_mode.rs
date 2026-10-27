//! Mode, acknowledgements and the once-a-second work of the host pipeline
//! (SCR-M2, SCR-M3, decisions C3 and C6): `SET_MODE` with its stream kind,
//! size and rate, another source, `FRAME_ACK`/`PREVIEW_ACK` for pacer and
//! rate control, and per second the loss counters, cadence and statistics.
use iroh::endpoint::Connection;

use super::{Pipeline, QualityMode, Stage, PREVIEW_EDGE};
use crate::screen::core::gop_stream::{loss_counters, now_us, path_of};
use crate::screen::core::host_backend::ScreenSource;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{adjust_quality, AckSample};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{FrameAck, HostStats};

impl Pipeline {
    /// `SET_MODE`: the stream kind, the size the viewer wants and the rate.
    pub(crate) fn set_mode(&mut self, kind: StreamKind, size: Option<FrameSize>, fps: Option<u16>) {
        let current = self.quality_policy.mode;
        self.set_quality_mode(current.update(kind, size, fps, current.sharpness, current.lossless));
    }

    pub(super) fn set_quality_mode(&mut self, mode: QualityMode) {
        let previous = self.quality_policy.mode;
        let previous_fps = self.max_fps;
        let previous_choice = self.choice;
        self.quality_policy.mode = mode;
        self.kind = mode.kind;
        self.choice = self.quality_choice();
        self.bounds = match (mode.kind, mode.size()) {
            (_, Some(size)) => size,
            (StreamKind::Preview, None) => FrameSize::new(PREVIEW_EDGE, PREVIEW_EDGE),
            _ => self.display,
        };
        self.max_fps = self.quality_policy.max_fps();
        self.preview_interval_us = 1_000_000 / u64::from(mode.preview_fps.max(1));
        // Temporary sharpness/preview caps never overwrite the requested full rate.
        self.quality.fps = if self.max_fps != previous_fps {
            self.max_fps
        } else {
            self.quality.fps.min(self.max_fps)
        };
        self.clock.set_fps(self.quality.fps);
        self.quality.size = self.fitted_size();
        if mode != previous || self.choice != previous_choice {
            self.invalidate_reference();
        }
        self.resize();
    }

    /// Replaces the source (`SET_MODE` with another source).
    pub(crate) fn set_source(&mut self, source: Box<dyn ScreenSource>) {
        // A detached old transition owns only the old Stage; polling can never reinstall it.
        self.capture_transition.take();
        self.capture_ready = None;
        self.capture_applied = None;
        self.capture_active = false;
        self.visible_rects = None;
        self.source_size = source.size();
        self.source_generation = source.reference_generation();
        self.frame_contract = self.negotiated_contract.map(|mut c| {
            c.hybrid_regions &= source.supports_frame_regions();
            c
        });
        self.stage = Some(Stage {
            memory: self.memory.clone(),
            source,
            encoder: None,
            encoder_book: None,
        });
        self.quality.size = self.fitted_size();
        self.invalidate_reference();
        if let Err(error) = self.start_capture_transition(self.capture_requested) {
            self.capture_ready = Some(Err(error));
        }
    }

    pub(crate) fn request_keyframe(&mut self) {
        if self.pacer.keyframe_due() && self.gop.is_none() {
            return;
        }
        self.invalidate_reference();
    }

    /// The id of the newest picture group the host opened (0: none yet).
    pub(crate) fn newest_gop(&self) -> u64 {
        self.next_gop_id.saturating_sub(1)
    }

    /// `FRAME_ACK` or `PREVIEW_ACK`.
    pub(crate) fn on_ack(&mut self, ack: FrameAck) -> bool {
        self.sender.ack(ack.gop_id, ack.frame_id);
        if self.preview_in_flight == Some(ack.frame_id) {
            self.preview_in_flight = None;
        }
        if let Some(sent) = self.pacer.on_ack(ack.gop_id, ack.frame_id) {
            let sample = AckSample {
                send_us: sent.sent_us,
                recv_us: ack.received_us,
                bytes: sent.bytes,
                keyframe: sent.keyframe,
            };
            self.rate.on_frame_acked(sample, now_us());
            return true;
        }
        false
    }

    /// Once a second: loss counters, cadence and statistics.
    pub(crate) fn second(&mut self, connection: &Connection) -> HostStats {
        let (sent, lost) = loss_counters(connection);
        let _ = self.rate.on_loss_counters(sent, lost);
        let (path, _) = path_of(connection);
        self.rate.set_path(path);

        let fitted = self.fitted_size();
        let next = adjust_quality(self.quality, self.rate.target_bps(), fitted, self.max_fps);
        if next.fps != self.quality.fps {
            self.clock.set_fps(next.fps);
            self.drop_encoder();
        }
        self.quality = next;
        self.resize();
        let stats = HostStats {
            target_kbps: self.rate.target_bps() / 1000,
            sent_kbps: self.second_bytes * 8 / 1000,
            fps: self.second_frames,
            path,
            keyframes: self.totals.keyframes,
            dropped_before_encode: self.pacer.dropped_before_encode(),
            stream_resets: self.pacer.resets(),
            unacked_bytes: self.pacer.unacked_bytes(),
        };
        self.second_bytes = 0;
        self.second_frames = 0;
        stats
    }
}

/// A `KEYFRAME_REQUEST` for an older picture group than the newest one the
/// host opened is answered already: every new group starts with a key
/// frame (`Pipeline::tick` opens none without one). The viewer asks after a
/// broken group ("nach Reset", C3 „Schlüsselbild“), usually before the
/// next group's first record arrives; honouring it reset that next group
/// as well, which broke it again on the viewer: a reset per round trip.
pub(super) fn keyframe_request_stale(requested: Option<u64>, newest: u64) -> bool {
    requested.is_some_and(|gop| gop < newest)
}

#[cfg(test)]
mod keyframe_request_tests {
    use super::keyframe_request_stale;

    #[test]
    fn screen_metadata_runtime_task_keyframe_request_for_a_replaced_group_is_answered() {
        // Group 7 broke, the host already opened group 8 with a key frame.
        assert!(keyframe_request_stale(Some(7), 8));
        // A broken or undecodable current group still gets a new key frame.
        assert!(!keyframe_request_stale(Some(8), 8));
        // A joining or older viewer names no group: always honoured.
        assert!(!keyframe_request_stale(None, 8));
        assert!(!keyframe_request_stale(Some(0), 0));
    }
}
