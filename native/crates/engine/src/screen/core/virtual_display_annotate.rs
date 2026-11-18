//! What a session's report adds to the display's state (user rule of
//! 2026-11-18 „Wahl gilt“): the viewer's fixed size when the host set
//! another one, with the reason (`VirtualDisplayReport::requested`,
//! `adjusted`), and the monitor the host moved the session to
//! (`VirtualDisplayReport::source`). The opening report of `HELLO_OK` is
//! decided before the source is prepared, so a moved source reaches the
//! viewer in `HostStats.chain.virtual_display` (`;source=monitor:<id>`).
use super::super::pixel_convert::FrameSize;
use super::super::virtual_display::{adjust_reason, DriverLimit, VirtualDisplayReport};
use super::{lock, shared, Entry, SessionStart, SHARED};

/// The monitor the viewer chose and the one the host shows.
#[derive(Clone, Debug, Default)]
pub(super) struct SourceTrack {
    /// The viewer's monitor (`monitor:<id>` without the prefix): the
    /// opening source, then every source it chose with `SET_MODE`.
    chosen: Option<String>,
    /// The next prepared source is the host's choice (the opening, after
    /// `SET_DISPLAY`), not the viewer's.
    awaiting: bool,
    /// The monitor the host shows instead of `chosen`.
    moved: Option<String>,
}

impl SourceTrack {
    /// A session opens on `monitor`; the host decides its source next.
    pub(super) fn opened(monitor: Option<String>) -> Self {
        Self {
            chosen: monitor,
            awaiting: true,
            moved: None,
        }
    }

    /// `SET_DISPLAY`: the host decides the source next.
    pub(super) fn display_changed(&mut self) {
        self.awaiting = true;
    }

    /// `SET_MODE` with a source: the viewer's own choice comes next.
    pub(super) fn viewer_chooses(&mut self) {
        self.awaiting = false;
    }

    /// The session's source is `monitor` now (`None`: not a monitor).
    pub(super) fn prepared(&mut self, monitor: Option<&str>) {
        if std::mem::take(&mut self.awaiting) {
            self.moved = monitor
                .filter(|shown| self.chosen.as_deref() != Some(shown))
                .map(str::to_string);
        } else {
            self.chosen = monitor.map(str::to_string);
            self.moved = None;
        }
    }

    /// `report` with the moved source, if any.
    fn annotate(&self, report: VirtualDisplayReport) -> VirtualDisplayReport {
        match &self.moved {
            Some(shown) => {
                let chosen = self.chosen.as_ref().map(|id| format!("monitor:{id}"));
                report.with_source(chosen.as_deref(), &format!("monitor:{shown}"))
            }
            None => report,
        }
    }
}

/// `report` of the session `start` with its fixed size when the display
/// shows another one: `pending` when the display's change waits for the gap
/// between two changes.
pub(super) fn requested(
    report: VirtualDisplayReport,
    start: &SessionStart,
    limit: DriverLimit,
    pending: bool,
) -> VirtualDisplayReport {
    if start.fixed.is_none() {
        return report;
    }
    let deciding = {
        let state = lock(&shared().state);
        state.planner.owner_session() == Some(start.session.as_str())
    };
    with_reason(report, start.fixed, limit, deciding, pending)
}

/// `report` with the fixed size `fixed` and the reason the host set another
/// one (`deciding`: the session decides the display's size, as on Wayland
/// where each session offers its own).
pub(super) fn with_reason(
    report: VirtualDisplayReport,
    fixed: Option<FrameSize>,
    limit: DriverLimit,
    deciding: bool,
    pending: bool,
) -> VirtualDisplayReport {
    let Some(fixed) = fixed else {
        return report;
    };
    match adjust_reason(fixed, report.size(), limit, deciding, pending) {
        Some(why) => report.with_requested(fixed, why),
        None => report,
    }
}

/// `HostStats.chain.virtual_display` of a session: `None` for a session
/// that sent no wish.
pub fn chain_key(session: &str) -> Option<String> {
    let shared = SHARED.get()?;
    let state = lock(&shared.state);
    let report = match state.reports.get(session)? {
        Entry::Own(report) => report.clone(),
        Entry::Display => {
            let report = state.display.clone()?;
            let fixed = state
                .sessions
                .get(session)
                .and_then(|known| known.start.fixed);
            let deciding = state.planner.owner_session() == Some(session);
            let pending = state.planner.applied() != state.planner.target(state.limit);
            with_reason(report, fixed, state.limit, deciding, pending)
        }
    };
    let report = match state.sessions.get(session) {
        Some(known) => known.track.annotate(report),
        None => report,
    };
    Some(report.chain_key())
}

#[cfg(test)]
#[path = "wahlgilt_source_task_tests.rs"]
mod wahlgilt_source_task_tests;
