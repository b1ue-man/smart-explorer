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
    /// Whether a changed visible area needs a key frame (round 8, Codex
    /// 8/9): the tiles and hybrid regions send only what is visible, a plain
    /// video stream's reference holds the whole picture, when its stage
    /// takes a picture of a still screen for the new area.
    pub fn visible_needs_keyframe(&mut self) -> bool {
        !self.choice.codec.is_video()
            || self.frame_contract.is_some_and(|c| c.hybrid_regions)
            // Without a stage (a capture switch) it is not known whether the
            // next one takes a picture of a still screen: a key frame, as
            // before (round 9).
            || self.stage.as_mut().is_none_or(|stage| {
                stage
                    .source
                    .host_stage()
                    .is_some_and(|host| !host.pictures_a_moved_view())
            })
    }
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
        // Review 2026-11-19 (Gemini 13.1): a source that follows the window
        // takes the new size itself; the picture keeps its bounds until then,
        // so only the source's change opens a new encoder, not this one too.
        if !self.awaits_source(&mode, now_us()) {
            self.bounds = mode_bounds(&mode, self.display);
        }
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
        // Codex 5/8, 6/10: what the encoder is (codec, profile, encoder
        // stage) decides; the viewer's decoder stage and the negotiated
        // limits do not. A size the new limits no longer allow restarts in
        // `resize`, a higher allowed rate at the next tick.
        let encoder =
            |c: &super::super::negotiate::CodecChoice| (c.codec, c.profile, c.encoder_hardware);
        // A running encoder whose own size and opened rate exceed the new
        // macroblock rate (its level could exceed the new decoder's) is
        // replaced; a running size beyond the new size or pixel limits
        // restarts in `resize` (the target size shrinks with them).
        let beyond = self.running.as_ref().is_some_and(|(_, running)| {
            crate::screen::core::pacing::codec_cap(
                self.codec_fps,
                *running,
                self.choice.max_macroblocks_per_second,
            )
            .is_none_or(|fps| fps < self.codec_fps)
        });
        // Codex 10/6: the stream kind (full view or preview) is in the GOP
        // header, not in what the encoder is asked: a new kind alone opens a
        // group of that kind with a key frame of the running encoder (its
        // preview receipts reset, `new_group`); `resize` replaces it when
        // its size no longer fits.
        let same_kind = previous.with_kind(mode.kind);
        if !mode.same_but_rate(&same_kind)
            || encoder(&self.choice) != encoder(&previous_choice)
            || beyond
        {
            self.note_restart("mode or codec pairing changed (SET_MODE/SET_DECODERS)");
            self.invalidate_reference();
        } else if mode.kind != previous.kind {
            if self.gop.is_none() && self.pacer.frames_in_flight() == 0 {
                self.sender.reset();
                self.new_group();
                self.kept_encoder = self.running.is_some();
            } else {
                self.reset_stream();
            }
        }
        self.resize();
    }

    /// The source takes `size`, the full view's next size, itself (a
    /// virtual display of this session that follows the window, P5): until
    /// it has that size, at most `FOLLOW_SOURCE_US`, the picture keeps its
    /// bounds (`set_quality_mode`, `second`). Call it before the `SET_MODE`
    /// with that size reaches `set_quality_mode`.
    pub fn expect_source_size(&mut self, size: FrameSize) {
        self.quality_policy.follow_source = Some((size, now_us().saturating_add(FOLLOW_SOURCE_US)));
    }

    /// Whether the bounds of `mode` wait for the source (`expect_source_size`).
    fn awaits_source(&self, mode: &QualityMode, now: u64) -> bool {
        self.quality_policy
            .follow_source
            .is_some_and(|(size, until)| {
                mode.kind == StreamKind::Video
                    && mode.size() == Some(size)
                    && self.source_size != size
                    && now < until
            })
    }

    /// The bounds that waited for the source (`expect_source_size`) apply
    /// once the source has the size, before `sync_source_reference` sees it
    /// (one new encoder for both), or when the wait is over.
    pub(super) fn apply_followed_bounds(&mut self) {
        let Some((size, until)) = self.quality_policy.follow_source else {
            return;
        };
        let source = self
            .stage
            .as_ref()
            .map_or(self.source_size, |stage| stage.source.size());
        if source != size && now_us() < until {
            return;
        }
        self.quality_policy.follow_source = None;
        self.bounds = mode_bounds(&self.quality_policy.mode, self.display);
    }

    /// The byte budget the session's held-back control frames share with
    /// the pictures (Codex 6/8, `host_session_send.rs`).
    pub fn shared_budget(&self) -> crate::screen::core::tiles_rate::SharedBudget {
        self.rate.shared_budget()
    }

    /// The control stream wrote `bytes` (`Session::frame`, review
    /// 2026-11-19): they count in the statistics' sent rate and, under a
    /// user ceiling, leave the pictures that much less of it.
    pub fn book_control_bytes(&mut self, bytes: u64) {
        self.second_bytes = self.second_bytes.saturating_add(bytes);
        self.rate.on_control_bytes(now_us(), bytes);
    }

    /// The viewer's „Höchste Datenrate“; `None` is automatic.
    pub fn user_ceiling_bps(&self) -> Option<u64> {
        self.rate.user_ceiling_bps()
    }

    /// „Höchste Datenrate“ of the viewer (B-09); `None` is automatic.
    pub fn set_rate_ceiling(&mut self, ceiling_bps: Option<u64>) {
        self.work.set_live_ceiling(ceiling_bps);
        self.rate.set_ceiling(ceiling_bps);
        // Round 7 P7: the held control frames' rate follows at once, not at
        // the next picture (`stream_admit` sets it; its answer is unused).
        let _ = self
            .rate
            .stream_admit(now_us(), self.choice.codec.is_video());
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
            visible: None,
        });
        self.quality.size = self.fitted_size();
        self.note_restart("another source: new encoder");
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
            self.pacer
                .note_ack_age(arrived_us.saturating_sub(sent.sent_us));
            // The viewer has the picture; its `PRESENTED` follows (pacing).
            self.pacer.on_received(ack.frame_id, arrived_us);
            // B-36: on the ordered stream every earlier frame arrived as
            // well. Only a frame this host sent and still awaits counts: an
            // acknowledgement that names any other id confirms nothing.
            self.confirmed.acked(ack.gop_id, ack.frame_id);
            // Gemini 8/3.1: the encoder keeps a confirmed long-term reference.
            if let (Some(stage), Some(good)) = (self.stage.as_mut(), self.confirmed.last_good()) {
                super::capture::confirm_reference(stage, good);
            }
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
        // Held-back control frames booked in the shared budget count in the
        // sent rate as well (once: not in the control rate).
        self.second_bytes = self
            .second_bytes
            .saturating_add(self.rate.shared_budget().take_held_bytes());
        // The sound of the session counts with the control stream (Codex
        // 5/4): the pictures get the ceiling less both.
        if let Some(link) = self.backend.audio() {
            let fresh = self
                .rate
                .on_audio_total(now_us(), link.sent_bytes(connection));
            self.second_bytes = self.second_bytes.saturating_add(fresh);
        }
        // The control stream's share of the ceiling; above the whole ceiling
        // the viewer is told once (the pictures keep the floor).
        self.rate.update_control(now_us());
        if let Some((control, ceiling)) = self.rate.control_over_ceiling() {
            self.note_encoder(format!(
                "{}the control stream's {control} bit/s leave the pictures too little of the \
                 highest data rate {ceiling} bit/s",
                crate::screen::core::codec_reason_text::ENCODER_NOTICE
            ));
        }

        self.quality.fps = cadence_fps(self.max_fps, self.rate.encode_capacity_fps());
        self.clock.set_fps(self.quality.fps);
        // A source that changed its size since the last tick opens its one
        // new encoder here (`sync_source_reference`), not a resize first.
        self.apply_followed_bounds();
        self.sync_source_reference();
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

/// How long the picture keeps its bounds for a source that follows the
/// window (`Pipeline::expect_source_size`): the virtual display changes its
/// size at most once per `MIN_RESIZE_GAP_MS` (1 s), and the capture then
/// reports it [assumption: within 2 s more; not measured].
const FOLLOW_SOURCE_US: u64 = 3_000_000;

/// The bounds of `mode`: its size, a preview's edge or the display.
fn mode_bounds(mode: &QualityMode, display: FrameSize) -> FrameSize {
    match (mode.kind, mode.size()) {
        (_, Some(size)) => size,
        (StreamKind::Preview, None) => FrameSize::new(PREVIEW_EDGE, PREVIEW_EDGE),
        _ => display,
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
