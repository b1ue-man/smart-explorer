//! Which changes of the route and of the viewer's visible area need a new
//! reference (a new encoder) and which keep the running one (review
//! 2026-11-19, V1/V2: about one new encoder a second with a GUI viewer).
use super::PipelineActor;
use crate::screen::core::visibility::VisibleRect;
use crate::share::route_state::RouteClass;
use std::io;

impl PipelineActor {
    /// A new route (its generation, a pause, another class) restarts the
    /// stream with a new reference. Whether the application may send is no
    /// part of it: the pipeline asks that before every picture
    /// (`Pipeline::tick_epoch`), and a route that stops the application
    /// counts a pause (`pause_epoch`, `route_policy::set_state`,
    /// `route_monitor_network_wait::pause`), so only its resumption changes
    /// `application_allowed` alone, which needs no new encoder.
    pub(super) fn watch_route(&mut self) {
        let route = crate::share::route_monitor::snapshot(&self.connection).map(|route| {
            (
                route.generation,
                route.pause_epoch,
                route.class,
                route.application_allowed,
            )
        });
        if route == self.route {
            return;
        }
        type Route = Option<(u64, u64, Option<RouteClass>, bool)>;
        let path = |route: Route| route.map(|(g, p, c, _)| (g, p, c));
        let moved = path(route) != path(self.route);
        self.route = route;
        if moved {
            self.interrupt();
            self.pending.invalidate = true;
        }
    }

    /// The negotiated contract carries hybrid regions: their encoder keeps
    /// per-region state bound to the visible area, so a change of it starts
    /// a new reference (`Session::visibility`).
    pub fn hybrid_regions(&self) -> bool {
        self.accepted
            .is_some_and(|contract| contract.hybrid_regions)
    }

    /// A visible view whose visible area changed and stays visible: the
    /// rectangles go to the next picture, the encoder and the capture stay.
    /// The viewer drops its shown picture with every report and waits for a
    /// picture at or after the acknowledged floor (gui `ResumeGate`), and a
    /// still screen gives none (`Captured::Unchanged`), so the next picture
    /// is a key frame of the running encoder (as for `KEYFRAME_REQUEST`).
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
        self.pending.keyframe = true;
        self.pending.confirm = true;
        self.poll();
        Ok(())
    }
}
