//! Which changes of the route and of the viewer's visible area need a new
//! reference (a new encoder) and which keep the running one (review
//! 2026-11-19, V1/V2: about one new encoder a second with a GUI viewer).
use super::{Connection, FrameSize, PipelineActor, StreamKind};
use crate::screen::core::visibility::VisibleRect;
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
            self.interrupt();
            self.pending.invalidate = true;
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
}
