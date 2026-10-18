//! Mode, acknowledgements and the once-a-second work of the host pipeline
//! (SCR-M2, SCR-M3, decisions C3 and C6): `SET_MODE` with its stream kind,
//! size and rate, another source, `FRAME_ACK`/`PREVIEW_ACK` for pacer and
//! rate control, and per second the loss counters, the relay guard of H.264
//! key frames, the quality ladder and the statistics.
use iroh::endpoint::Connection;

use super::{Pipeline, Stage, PREVIEW_EDGE};
use crate::screen::core::gop_stream::{loss_counters, now_us, path_of};
use crate::screen::core::host_backend::ScreenSource;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pacing::RELAY_QUEUE_BYTES;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{adjust_quality, AckSample, PathKind};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{FrameAck, HostStats};

impl Pipeline {
    /// `SET_MODE`: the stream kind, the size the viewer wants and the rate.
    pub(crate) fn set_mode(&mut self, kind: StreamKind, size: Option<FrameSize>, fps: Option<u16>) {
        if kind != self.kind {
            self.kind = kind;
            self.preview_in_flight = None;
            self.drop_encoder();
        }
        self.bounds = match (kind, size) {
            (_, Some(size)) => size,
            (StreamKind::Preview, None) => FrameSize::new(PREVIEW_EDGE, PREVIEW_EDGE),
            _ => self.display,
        };
        if let Some(fps) = fps {
            let fps = u32::from(fps).max(1);
            if kind == StreamKind::Preview {
                self.preview_interval_us = 1_000_000 / u64::from(fps);
            } else {
                self.max_fps = fps;
                self.quality.fps = self.quality.fps.min(fps);
                self.clock.set_fps(self.quality.fps);
            }
        }
        self.quality.size = self.fitted_size();
        self.resize();
    }

    /// Replaces the source (`SET_MODE` with another source).
    pub(crate) fn set_source(&mut self, source: Box<dyn ScreenSource>) {
        self.stage = Some(Stage {
            source,
            encoder: None,
        });
        self.quality.size = self.fitted_size();
        self.drop_encoder();
    }

    pub(crate) fn request_keyframe(&mut self) {
        self.pacer.request_keyframe();
    }

    /// `FRAME_ACK` or `PREVIEW_ACK`.
    pub(crate) fn on_ack(&mut self, ack: FrameAck) {
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
        }
    }

    /// Once a second: loss counters, relay guard, quality ladder and the
    /// statistics.
    pub(crate) fn second(&mut self, connection: &Connection) -> HostStats {
        let (sent, lost) = loss_counters(connection);
        let _ = self.rate.on_loss_counters(sent, lost);
        let (path, _) = path_of(connection);
        self.rate.set_path(path);
        self.relay_guard(path);
        let fitted = self.fitted_size();
        let next = adjust_quality(self.quality, self.rate.target_bps(), fitted, self.max_fps);
        if next.fps != self.quality.fps {
            self.clock.set_fps(next.fps);
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

    /// The relay drops what exceeds its queue per destination (NR N19): an
    /// H.264 key frame larger than `RELAY_QUEUE_BYTES` would lose packets
    /// there every time, so the next picture groups get 3/4 of the size
    /// until the path is direct again.
    fn relay_guard(&mut self, path: PathKind) {
        if path != PathKind::Relay {
            self.relay_limit = None;
            self.last_key_bytes = 0;
            return;
        }
        if self.choice.codec == CodecKind::H264 && self.last_key_bytes > RELAY_QUEUE_BYTES {
            let current = self
                .running
                .as_ref()
                .map_or_else(|| self.target_size(), |(_, size)| *size);
            self.relay_limit = Some(current.scale(3, 4));
            self.last_key_bytes = 0;
        }
    }
}
