//! Mode, acknowledgements and the once-a-second work of the host pipeline
//! (SCR-M2, SCR-M3, decisions C3 and C6): `SET_MODE` with its stream kind,
//! size and rate, another source, `FRAME_ACK`/`PREVIEW_ACK` for pacer and
//! rate control, and per second the loss counters, cadence and statistics.
//! The cadence follows the wanted rate and the measured encoding capacity,
//! never the bit rate, and a change of it keeps the encoder (plan
//! 2026-11-14 „Bildkette“ B-02, B-03).
use iroh::endpoint::Connection;

use super::{Pipeline, QualityMode, Stage, PREVIEW_EDGE};
use crate::screen::core::chain_meter::RateFacts;
use crate::screen::core::gop_stream::{loss_counters, now_us, path_of};
use crate::screen::core::host_backend::ScreenSource;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{cadence_fps, AckSample};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{FrameAck, HostStats};

impl Pipeline {
    /// `SET_MODE`: the stream kind, the size the viewer wants and the rate.
    pub fn set_mode(&mut self, kind: StreamKind, size: Option<FrameSize>, fps: Option<u16>) {
        let current = self.quality_policy.mode;
        self.set_quality_mode(current.update(kind, size, fps, current.sharpness, current.lossless));
    }

    pub(super) fn set_quality_mode(&mut self, mut mode: QualityMode) {
        let previous = self.quality_policy.mode;
        let previous_fps = self.max_fps;
        let previous_choice = self.choice;
        // The viewer's codec choice comes with the opening and `SET_DECODERS`
        // (plan 2026-11-18); a mode copied before it keeps the current one.
        mode.chosen = previous.chosen;
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
            cadence_fps(self.max_fps, self.rate.encode_capacity_fps())
        } else {
            self.quality.fps.min(self.max_fps)
        };
        self.clock.set_fps(self.quality.fps);
        self.quality.size = self.fitted_size();
        // A new rate alone keeps the encoder (B-03); a rate above the one it
        // was opened with restarts it at the next tick (`Pipeline::tick`).
        if !mode.same_but_rate(&previous) || self.choice != previous_choice {
            self.note_restart("the viewer's mode or codec changed (SET_MODE): new encoder");
            self.invalidate_reference();
        }
        self.resize();
    }

    /// „Höchste Datenrate“ of the viewer (B-09); `None` is automatic.
    pub fn set_rate_ceiling(&mut self, ceiling_bps: Option<u64>) {
        self.rate.set_ceiling(ceiling_bps);
    }

    /// Replaces the source (`SET_MODE` with another source).
    pub fn set_source(&mut self, source: Box<dyn ScreenSource>) {
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
        let refused = self
            .stage
            .as_mut()
            .map_or_else(Vec::new, |stage| std::mem::take(&mut stage.refused));
        self.stage = Some(Stage {
            memory: self.memory.clone(),
            source,
            encoder: None,
            encoder_book: None,
            refused,
            chain: self.chain.clone(),
        });
        self.quality.size = self.fitted_size();
        self.invalidate_reference();
        if let Err(error) = self.start_capture_transition(self.capture_requested) {
            self.capture_ready = Some(Err(error));
        }
    }

    pub fn request_keyframe(&mut self) {
        if self.pacer.keyframe_due() && self.gop.is_none() {
            return;
        }
        // A key frame of the running encoder answers it; nothing else
        // changed, so the encoder is not opened anew (2026-11-18).
        if self.gop.is_none() && self.pacer.frames_in_flight() == 0 {
            self.sender.reset();
            self.new_group();
            self.kept_encoder = self.running.is_some();
        } else {
            self.reset_stream();
        }
    }

    /// The id of the newest picture group the host opened with a key frame
    /// (0: none yet). A group that continues the chain (B-36) answers no
    /// `KEYFRAME_REQUEST`: a viewer whose chain broke cannot decode it.
    pub fn newest_gop(&self) -> u64 {
        self.key_gop
    }

    /// `FRAME_ACK` or `PREVIEW_ACK`.
    pub fn on_ack(&mut self, ack: FrameAck) -> bool {
        self.on_ack_at(ack, now_us())
    }

    /// `FRAME_ACK` or `PREVIEW_ACK` that arrived at `arrived_us` (one queued
    /// while a picture operation ran keeps its arrival time, BK3b).
    pub fn on_ack_at(&mut self, ack: FrameAck, arrived_us: u64) -> bool {
        self.sender.ack(ack.gop_id, ack.frame_id);
        if self.preview_in_flight == Some(ack.frame_id) {
            self.preview_in_flight = None;
        }
        if let Some(sent) = self.pacer.on_ack(ack.gop_id, ack.frame_id) {
            // B-36: on the ordered stream every earlier frame arrived as
            // well. Only a frame this host sent and still awaits counts: an
            // acknowledgement that names any other id confirms nothing.
            self.confirmed.acked(ack.gop_id, ack.frame_id);
            // Station `network` (BK0), in raw clocks; the viewer applies its offset.
            super::super::chain_meter::lock(&self.chain).on_network(sent.sent_us, ack.received_us);
            let sample = AckSample {
                send_us: sent.sent_us,
                recv_us: ack.received_us,
                bytes: sent.bytes,
                keyframe: sent.keyframe,
            };
            self.rate.on_frame_acked(sample, arrived_us);
            return true;
        }
        false
    }

    /// Once a second: loss counters, back pressure, the remembered rate,
    /// cadence and statistics. The cadence is the wanted rate, at most what
    /// the encoding of this session carries (B-02); the bit rate does not
    /// enter it, and a new cadence keeps the encoder (B-03).
    pub fn second(&mut self, connection: &Connection) -> HostStats {
        let (sent, lost) = loss_counters(connection);
        let _ = self.rate.on_loss_counters(sent, lost);
        let (path, _) = path_of(connection);
        self.rate.set_path(path);
        if let Some(events) = congestion_events(connection) {
            self.rate.on_congestion_events(events, now_us());
        }
        self.rate.remember();

        self.quality.fps = cadence_fps(self.max_fps, self.rate.encode_capacity_fps());
        self.clock.set_fps(self.quality.fps);
        self.quality.size = self.fitted_size();
        self.resize();
        // BK0: stages, limits and station times; the session decides
        // whether the station times leave the host (`chain_meter::publish`).
        let rates = RateFacts {
            target_fps: self.quality.fps,
            wanted_fps: self.max_fps,
            capacity_fps: self.rate.encode_capacity_fps(),
            sent_fps: self.second_frames,
            target_bps: self.rate.target_bps(),
            ceiling_bps: self.rate.ceiling_bps(),
            user_ceiling_bps: self.rate.user_ceiling_bps(),
            back_pressure: self.rate.back_pressure(now_us()),
            resets: self.pacer.resets(),
            // Plan 2026-11-17: the reason, the encoder's rate and its honour.
            limit: Some(self.rate.limit()),
            encoder_bps: Some(self.rate.encoder_bps()),
            honour: Some(self.rate.honour()),
        };
        let (chain, stations) = {
            let meter = super::super::chain_meter::lock(&self.chain);
            (meter.report(&rates), meter.stations())
        };
        let stats = HostStats {
            target_kbps: self.rate.target_bps() / 1000,
            sent_kbps: self.second_bytes * 8 / 1000,
            fps: self.second_frames,
            path,
            keyframes: self.totals.keyframes,
            dropped_before_encode: self.pacer.dropped_before_encode(),
            stream_resets: self.pacer.resets(),
            unacked_bytes: self.pacer.unacked_bytes(),
            frame_timing: Some(super::super::frame_timing::FRAME_TIMING_VERSION),
            chain: Some(chain),
            stations,
        };
        self.second_bytes = 0;
        self.second_frames = 0;
        stats
    }
}

/// Cumulative congestion events of the selected path; `None` without one.
fn congestion_events(connection: &Connection) -> Option<u64> {
    let paths = connection.paths();
    let selected = paths.iter().find(|path| path.is_selected())?;
    Some(selected.stats().congestion_events)
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
