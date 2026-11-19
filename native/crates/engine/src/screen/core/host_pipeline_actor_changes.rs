//! Which changes of the route and of the viewer's visible area need a new
//! reference (a new encoder) and which keep the running one (review
//! 2026-11-19, V1/V2: about one new encoder a second with a GUI viewer).
use super::{Connection, FrameSize, PipelineActor, StreamKind};
use crate::screen::core::visibility::VisibleRect;
use crate::screen::core::wire_control::KeyframeReason;
use crate::share::route_state::RouteClass;
use std::io;

pub(super) type Route = Option<(u64, u64, Option<RouteClass>, bool)>;

/// The route of `connection` as the actor watches it. The actor starts with
/// the route it was made on: that one needs no new reference (Gemini 1).
pub(super) fn route_of(connection: &Connection) -> Route {
    crate::share::route_monitor::snapshot(connection).map(|route| {
        (
            route.generation,
            route.pause_epoch,
            route.class,
            route.application_allowed,
        )
    })
}

impl PipelineActor {
    /// A new route (its generation, a pause, another class) restarts the
    /// stream with a new reference. Whether the application may send is no
    /// part of it: the pipeline asks that before every picture
    /// (`Pipeline::tick_epoch`), and a route that stops the application
    /// counts a pause (`pause_epoch`, `route_policy::set_state`,
    /// `route_monitor_network_wait::pause`), so only its resumption changes
    /// `application_allowed` alone, which needs no new encoder.
    pub(super) fn watch_route(&mut self) {
        let route = route_of(&self.connection);
        if route == self.route {
            return;
        }
        let path = |route: Route| route.map(|(g, p, c, _)| (g, p, c));
        let moved = path(route) != path(self.route);
        self.route = route;
        if moved {
            // Codex 5/9: the transport changed, not the picture: the stream
            // starts anew on the new path with a key frame of the running
            // encoder (`reset_stream`), which keeps the reference input
            // binds to, as a key frame does (`request_keyframe`).
            let before = self.work.reference();
            self.interrupt();
            if !self.blocked && self.work.reference() == before + 1 {
                self.facts.reference = self.work.reference();
            }
            self.pending.route_reset = true;
        }
    }

    /// `SET_MODE`: a changed stream kind, sharpness, losslessness or chosen
    /// codec ends the running operation; a new rate or size does not.
    pub fn set_mode(
        &mut self,
        kind: StreamKind,
        size: Option<FrameSize>,
        fps: Option<u16>,
        sharpness: bool,
        lossless: bool,
    ) {
        let mode = self.mode.update(kind, size, fps, sharpness, lossless);
        if self.mode == mode {
            return;
        }
        // A new rate or size alone waits for the running operation (review
        // O1): the pipeline takes it in `set_quality_mode`/`resize`.
        let more = (mode.kind, mode.sharpness, mode.lossless, mode.chosen)
            != (
                self.mode.kind,
                self.mode.sharpness,
                self.mode.lossless,
                self.mode.chosen,
            );
        self.mode = mode;
        self.presence.set_full(mode.kind == StreamKind::Video);
        if more {
            self.interrupt();
        }
        self.pending.mode = Some(mode);
        self.poll();
    }
    /// The virtual display follows the window to `size`
    /// (`virtual_display_host::resize`): the picture keeps its bounds until
    /// the source has it (`Pipeline::expect_source_size`), so one encoder
    /// opens for both. Before the `SET_MODE` with that size; applied before
    /// it (`apply`), without an interrupt.
    pub fn expect_source_size(&mut self, size: FrameSize) {
        self.pending.expect = Some(size);
        self.poll();
    }

    /// A visible view whose visible area changed and stays visible: the
    /// rectangles go to the next picture, the encoder and the capture stay.
    /// The viewer drops its shown picture with every report and waits for a
    /// picture at or after the acknowledged floor (gui `ResumeGate`), and a
    /// still screen gives none (`Captured::Unchanged`), so the next picture
    /// is a key frame of the running encoder, inside the open group (Codex
    /// 3/4: a new group cost a new stream and a new decoder on the viewer,
    /// which takes a key frame inside its group already).
    /// The capture's state is reported once more, which lets the session
    /// send the acknowledgement with the floor after this change.
    pub fn retarget_visible_rects(&mut self, rects: Option<Vec<VisibleRect>>) -> io::Result<()> {
        if self.stopped {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "pipeline stopped",
            ));
        }
        self.pending.visible = Some(rects);
        self.pending.refresh = true;
        self.pending.confirm = true;
        self.poll();
        Ok(())
    }

    /// Every `PRESENTED` of the viewer reaches the pacer (review 2026-11-19,
    /// round 5 G1), also one the session does not take for input (a hidden
    /// view, a pending visibility acknowledgement, another reference): what
    /// the viewer shows paces the pictures (`Pacer::viewer_behind`).
    pub fn note_presented(&mut self, frame: u64) {
        let now = crate::screen::core::gop_stream::now_us();
        match self.pipeline.as_mut() {
            Some(p) => p.pacer.on_presented(frame, now),
            None => {
                let newest = self
                    .pending
                    .presented
                    .map_or(frame, |(known, _)| known.max(frame));
                self.pending.presented = Some((newest, now));
            }
        }
    }

    /// The session wrote `bytes` on its control stream (review 2026-11-19):
    /// booked now, or after the running picture operation.
    pub fn book_control_bytes(&mut self, bytes: u64) {
        match self.pipeline.as_mut() {
            Some(p) => p.book_control_bytes(bytes),
            None => {
                self.pending.control_bytes = self.pending.control_bytes.saturating_add(bytes);
            }
        }
    }

    /// `KEYFRAME_REQUEST` naming the viewer's group `gop`. Review 2026-11-19
    /// (Codex 4/9): a decoder that failed or was changed on a stream that
    /// still arrives (`DecodeError`, `Join`) takes a key frame inside the
    /// open group, as a visibility change does (`retarget_visible_rects`);
    /// the stream, its bytes on the line and the viewer's reader stay. Only
    /// a broken stream (`Reset`) gets a new one.
    pub fn request_keyframe(&mut self, gop: Option<u64>, reason: KeyframeReason) {
        if self.facts.keyframe
            || self.facts.reference != self.work.reference()
            || super::super::mode::keyframe_request_stale(gop, self.facts.gop)
        {
            return;
        }
        if reason != KeyframeReason::Reset {
            self.pending.refresh = true;
            self.poll();
            return;
        }
        // A key frame keeps the reference input binds to (review 2026-11-19,
        // E1): the running operation ends, input goes on.
        let before = self.work.reference();
        self.work.interrupt();
        if !self.blocked && self.work.reference() == before + 1 {
            self.facts.reference = self.work.reference();
        }
        self.pending.keyframe = true;
        self.poll();
    }
}
