//! `SET_DISPLAY` on the host (plan 2026-11-17 P9, `set_display.rs`): a
//! session decides its display again with a new wish, without a new
//! session. Its claim and its physical mode are released; a physical
//! monitor that no session asks to change any more gets its own mode back
//! at once (not after the background thread's next turn), unless the new
//! wish changes it again anyway; then the session is decided as at its
//! opening, without the gap between two changes (the viewer chose it). The
//! host session prepares its monitor source again afterwards
//! (`host_visibility.rs`). Also the fixed size at a physical monitor
//! (`physical`), which the opening uses as well.
use std::sync::Arc;

use super::super::pixel_convert::FrameSize;
use super::super::set_display::{physical_size, Shown};
use super::super::virtual_display::{
    VirtualDisplayChoice, VirtualDisplayReport, VirtualDisplayState, VirtualDisplayWish,
    REASON_OFF, REASON_PHYSICAL,
};
use super::SessionStart;
use super::{
    decide, ensure_thread, lock, shared, DisplayProbe, Shared, VirtualDisplayDriver, SHARED,
};

/// Reason of `Inactive` when the physical monitor lists no mode for the
/// fixed size or refused it.
pub const REASON_PHYSICAL_MODE_REFUSED: &str = "physical_mode_refused";

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
        let context = known.clone();
        state.planner.release(session);
        state.reports.remove(session);
        state.portal.remove(&old_capture);
        state.physical_holders.remove(session);
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
/// monitor, `None` for a window, a family or the test picture.
pub fn source_changed(session: &str, monitor: Option<String>) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    if let Some(known) = lock(&shared.state).sessions.get_mut(session) {
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
            VirtualDisplayReport::new(VirtualDisplayState::Inactive, Some(shown), &probe.driver)
                .with_reason(reason)
        }
        Err(error) => {
            eprintln!(
                "virtual display: physical monitor {}x{}: {error}",
                size.width, size.height
            );
            VirtualDisplayReport::new(VirtualDisplayState::Inactive, probe.current, &probe.driver)
                .with_reason(REASON_PHYSICAL_MODE_REFUSED)
        }
    }
}
