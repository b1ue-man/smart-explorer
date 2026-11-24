//! `SET_DISPLAY` on the host (plan 2026-11-17 P9, `set_display.rs`): a
//! session decides its display again with a new wish, without a new
//! session. Its claim and its physical mode are released; a physical
//! monitor that no session asks to change any more gets its own mode back
//! at once (not after the background thread's next turn), unless the new
//! wish changes it again anyway; then the session is decided as at its
//! opening, without the gap between two changes (the viewer chose it). The
//! host session prepares its monitor source again afterwards
//! (`host_visibility.rs`). Also the fixed size at a physical monitor
//! (`physical`), which the opening uses as well, and the end of a driver
//! setup that sessions waited for (Windows at use time, user rule
//! 2026-11-24): after success each of them decides again with its choice
//! as with `SET_DISPLAY` (`resume_due`, asked by the session on every
//! viewer ping, so the session's monitor source follows), after a decline
//! or a failure its report names the cause.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::super::pixel_convert::FrameSize;
use super::super::set_display::{physical_size, Shown};
pub use super::super::virtual_display::REASON_PHYSICAL_MODE_REFUSED;
use super::super::virtual_display::{
    VirtualDisplayChoice, VirtualDisplayReport, VirtualDisplayState, VirtualDisplayWish,
    ADJUST_PHYSICAL_MODE, ADJUST_REFUSED, REASON_OFF, REASON_PHYSICAL,
};
use super::SessionStart;
use super::{
    decide, ensure_thread, lock, shared, DisplayProbe, Entry, SetupProgress, Shared,
    VirtualDisplayDriver, SHARED,
};

/// The sessions around a driver setup they started or joined.
#[derive(Default)]
pub(super) struct SetupWait {
    /// Waiting for the end of the running setup.
    waiting: HashSet<String>,
    /// How the setup ended for a session that waited (`resume_due` takes it).
    ended: HashMap<String, SetupProgress>,
    /// Deciding again after the end: they start no new setup then (the end
    /// of one setup never starts the next one by itself).
    resuming: HashSet<String>,
}

impl SetupWait {
    /// The session ended.
    pub(super) fn leave(&mut self, session: &str) {
        self.waiting.remove(session);
        self.ended.remove(session);
        self.resuming.remove(session);
    }
}

/// Whether `session` decides again after the setup it waited for ended
/// (once; `decide` asks first).
pub(super) fn resumed(session: &str) -> bool {
    lock(&shared().state).setup.resuming.remove(session)
}

/// Starts or joins the driver setup for `session`; a running one keeps the
/// session waiting for its end (registered before it starts, so no end
/// passes unseen).
pub(super) fn request_setup(
    driver: &Arc<dyn VirtualDisplayDriver>,
    session: &str,
) -> SetupProgress {
    lock(&shared().state)
        .setup
        .waiting
        .insert(session.to_string());
    let progress = driver.request_setup();
    if progress != SetupProgress::Running {
        lock(&shared().state).setup.waiting.remove(session);
    }
    progress
}

/// A driver setup that sessions waited for ended (`Done`, or `Failed` with
/// the reason; Windows at use time). Each waiting session takes it with
/// `resume_due`.
pub fn setup_ended(progress: SetupProgress) {
    let mut state = lock(&shared().state);
    let waiting: Vec<String> = state.setup.waiting.drain().collect();
    for session in waiting {
        state.setup.ended.insert(session, progress.clone());
    }
}

/// The session's share of a setup that ended: after success the choice it
/// decides its display again with (as `SET_DISPLAY`, so its monitor source
/// follows the virtual display); after a failure its report names the
/// reason (the statistics show it) and `None`. `None` without an end.
pub fn resume_due(session: &str) -> Option<VirtualDisplayChoice> {
    let shared = SHARED.get()?;
    let mut state = lock(&shared.state);
    let ended = state.setup.ended.remove(session)?;
    let start = state.sessions.get(session)?.start.clone();
    match ended {
        SetupProgress::Failed(reason) => {
            let (size, driver) = match state.reports.get(session) {
                Some(Entry::Own(report)) => (report.size(), report.driver.clone()),
                _ => (None, String::new()),
            };
            let report = VirtualDisplayReport::new(VirtualDisplayState::Unsupported, size, &driver)
                .with_reason(&reason);
            state
                .reports
                .insert(session.to_string(), Entry::Own(report));
            None
        }
        SetupProgress::Running => None,
        SetupProgress::Done | SetupProgress::AfterRestart => {
            let wish = start.wish?;
            state.setup.resuming.insert(session.to_string());
            Some(VirtualDisplayChoice {
                wish,
                size: start.fixed,
            })
        }
    }
}

/// The answer of `switch`: the session's new report and the monitor it
/// showed.
#[derive(Clone, Debug)]
pub struct Switched {
    pub report: VirtualDisplayReport,
    /// The id of the session's monitor before the change.
    pub monitor: Option<String>,
}

impl Switched {
    /// What the display shows the session now.
    pub fn shown(&self) -> Shown {
        if self.report.state == VirtualDisplayState::Active {
            Shown::Virtual
        } else {
            Shown::Physical
        }
    }
}

/// Decides `session` again with `choice` (blocking); `capture` names the
/// capture session its source is prepared in next (the Wayland portal gets
/// the size by it). `None` for a session that sent no wish or that the
/// host's driver does not serve: the viewer never got `live` for it.
pub fn switch(session: &str, choice: VirtualDisplayChoice, capture: &str) -> Option<Switched> {
    let shared = SHARED.get()?;
    let (context, physical_now) = {
        let mut state = lock(&shared.state);
        let known = state.sessions.get_mut(session)?;
        let old_capture = std::mem::replace(&mut known.start.capture, capture.to_string());
        known.start.wish = Some(choice.wish);
        known.start.fixed = choice.size;
        // The source prepared next is the host's choice for this display.
        known.track.display_changed();
        let context = known.clone();
        state.planner.release(session);
        state.reports.remove(session);
        state.portal.remove(&old_capture);
        state.physical_holders.remove(session);
        // The new choice decides again; a setup still running is joined.
        state.setup.waiting.remove(session);
        state.setup.ended.remove(session);
        (context, state.physical)
    };
    let claims_again = choice.wish == VirtualDisplayWish::Request
        || physical_size(choice.wish, physical_now, choice.size).is_some();
    if !claims_again {
        give_back_now(shared);
    }
    let mut report = decide(&context.driver, &context.start, true);
    report.live = true;
    Some(Switched {
        report,
        monitor: context.start.monitor,
    })
}

/// The session's source changed (`SET_MODE`) or was prepared again: its
/// monitor, `None` for a window, a family or the test picture. After the
/// opening and after `SET_DISPLAY` the source is the host's choice: a
/// monitor other than the viewer's is reported as moved
/// (`VirtualDisplayReport::source`).
pub fn source_changed(session: &str, monitor: Option<String>) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    if let Some(known) = lock(&shared.state).sessions.get_mut(session) {
        known.track.prepared(monitor.as_deref());
        known.start.monitor = monitor;
    }
}

/// The session's next source preparation runs in the capture session
/// `capture` (a Wayland session's portal size moves with it).
pub fn rebind_capture(session: &str, capture: &str) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    let mut state = lock(&shared.state);
    let Some(known) = state.sessions.get_mut(session) else {
        return;
    };
    // `SET_MODE` with a source: the viewer chose it.
    known.track.viewer_chooses();
    let old = std::mem::replace(&mut known.start.capture, capture.to_string());
    if let Some(size) = state.portal.remove(&old) {
        state.portal.insert(capture.to_string(), size);
    }
}

/// Gives a physical monitor its own mode back now when no session asks to
/// change it any more.
fn give_back_now(shared: &'static Shared) {
    let driver = {
        let mut state = lock(&shared.state);
        if !state.give_back_due() {
            return;
        }
        state.restore = false;
        state.driver.clone()
    };
    if let Some(driver) = driver {
        let _turn = lock(&shared.turn);
        if let Err(error) = driver.restore() {
            eprintln!("virtual display: the monitor's own mode: {error}");
        }
    }
    let mut state = lock(&shared.state);
    state.planner.forget();
    state.display = None;
    state.physical_mode = false;
}

/// A fixed size at a physical monitor: the mode the monitor lists nearest
/// to it (`VirtualDisplayDriver::physical_mode`); the monitor gets its own
/// mode back after the last session that asked for one.
pub(super) fn physical(
    driver: &Arc<dyn VirtualDisplayDriver>,
    start: &SessionStart,
    probe: &DisplayProbe,
    size: FrameSize,
) -> VirtualDisplayReport {
    let shared = shared();
    let reason = if start.wish == Some(VirtualDisplayWish::Off) {
        REASON_OFF
    } else {
        REASON_PHYSICAL
    };
    let result = {
        let _turn = lock(&shared.turn);
        driver.physical_mode(size)
    };
    let mut state = lock(&shared.state);
    state.driver = Some(driver.clone());
    state.driver_name = probe.driver.clone();
    match result {
        Ok(shown) => {
            state.physical_holders.insert(start.session.clone());
            // The display shows no mode of se's any more.
            state.planner.forget();
            state.display = None;
            state.physical_mode = true;
            state.restore = false;
            drop(state);
            // The background thread gives the mode back after the last session.
            ensure_thread(shared);
            // „Wahl gilt“: a mode other than the fixed size is reported.
            VirtualDisplayReport::new(VirtualDisplayState::Inactive, Some(shown), &probe.driver)
                .with_reason(reason)
                .with_requested(size, ADJUST_PHYSICAL_MODE)
        }
        Err(error) => {
            eprintln!(
                "virtual display: physical monitor {}x{}: {error}",
                size.width, size.height
            );
            VirtualDisplayReport::new(VirtualDisplayState::Inactive, probe.current, &probe.driver)
                .with_reason(REASON_PHYSICAL_MODE_REFUSED)
                .with_requested(size, &format!("{ADJUST_REFUSED}: {error}"))
        }
    }
}
