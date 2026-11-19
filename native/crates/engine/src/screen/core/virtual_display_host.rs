//! Runs the virtual display for the sessions of this host (plan 2026-11-17
//! P5). A view or control session of a viewer that sent a wish starts here
//! before its source is prepared (`host_server_start.rs`): the platform
//! probes its display (`VirtualDisplayDriver`), the rules of
//! `virtual_display` decide, the size of the hello (or the viewer's fixed
//! size) is applied at once, and the answer goes into `HELLO_OK`. Every
//! later `SET_MODE` size (`host_visibility.rs`) reaches one background
//! thread that applies the size of the deciding session at most once a
//! second. The end of a session releases its claim; the size stays, except
//! that a physical monitor gets its own mode back after the last session
//! that asked for se's display there (`request`).
//! Without a stage-1 path (no display modes, or a driver limit below the
//! viewer's size) and without a physical monitor the driver setup with one
//! elevation is started (stage 2), once per version and only for a viewer
//! that may steer this screen. On Wayland the compositor makes the display:
//! the size reaches the host process with its preparation (`portal_size`).
//! `SET_DISPLAY` (P9, `set_display.rs`) decides a session again with its new
//! wish within the session (`virtual_display_switch.rs`); a fixed size at a
//! physical monitor sets a mode the monitor lists (`physical_size`), given
//! back like se's modes after the last session.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use super::pixel_convert::FrameSize;
use super::set_display::physical_size;
use super::virtual_display::{
    fit_mode, needs_virtual, setup_allowed, DriverLimit, Due, Planner, VirtualDisplayReport,
    VirtualDisplayState, VirtualDisplayWish, REASON_OFF, REASON_PHYSICAL,
};
use super::wire_hello::SessionMode;

#[path = "virtual_display_platform.rs"]
mod platform;
use platform::unsupported;
pub use platform::{capability, DisplayFacts, DisplayProbe, SetupProgress, VirtualDisplayDriver};

#[path = "virtual_display_switch.rs"]
mod switch;
use switch::physical;
pub use switch::{rebind_capture, source_changed, switch, Switched, REASON_PHYSICAL_MODE_REFUSED};

#[path = "virtual_display_annotate.rs"]
mod annotate;
pub use annotate::chain_key;

/// What a starting session brings.
#[derive(Clone, Debug)]
pub struct SessionStart {
    /// The registry id of the session (`SessionHandle::id`).
    pub session: String,
    /// The capture session of its preparation (`SessionAdmission::session`):
    /// the host process of a Wayland session gets the size by it.
    pub capture: String,
    pub mode: SessionMode,
    pub metadata_only: bool,
    pub wish: Option<VirtualDisplayWish>,
    /// The viewer's fixed size („Auflösung“); `None` follows its window.
    pub fixed: Option<FrameSize>,
    pub display_size: Option<FrameSize>,
    /// The viewer may steer this screen (stage 2 is allowed for it).
    pub may_setup: bool,
    /// The id of the session's monitor (`monitor:<id>`); `SET_DISPLAY`
    /// keeps the session on its monitor where the display allows it.
    pub monitor: Option<String>,
}

/// The answer for `HELLO_OK` and what the session holds until its end.
#[derive(Default)]
pub struct Started {
    pub report: Option<VirtualDisplayReport>,
    pub claim: Option<DisplayClaim>,
}

/// Held by a session that sent a wish; dropping it releases the claim (the
/// size stays), its report and its Wayland size.
pub struct DisplayClaim {
    session: String,
    capture: String,
}

impl Drop for DisplayClaim {
    fn drop(&mut self) {
        let shared = shared();
        let mut state = lock(&shared.state);
        state.planner.release(&self.session);
        state.reports.remove(&self.session);
        // The capture of a session that changed its display is the last
        // one `rebind_capture` named.
        let capture = state
            .sessions
            .remove(&self.session)
            .map_or_else(|| self.capture.clone(), |known| known.start.capture);
        state.portal.remove(&capture);
        state.portal.remove(&self.capture);
        state.physical_holders.remove(&self.session);
        if state.give_back_due() {
            state.restore = true;
        }
        drop(state);
        shared.changed.notify_all();
    }
}

/// What deciding a session again needs (`SET_DISPLAY`).
#[derive(Clone)]
struct SessionContext {
    driver: Arc<dyn VirtualDisplayDriver>,
    start: SessionStart,
    /// The viewer's monitor and the one the host moved it to.
    track: annotate::SourceTrack,
    /// The session shows a source the viewer chose, not the display (W1).
    own_source: bool,
}

/// The report of one session: its own, or the display's (a session whose
/// size the display follows).
#[derive(Clone)]
enum Entry {
    Own(VirtualDisplayReport),
    Display,
}

struct HostState {
    planner: Planner,
    driver: Option<Arc<dyn VirtualDisplayDriver>>,
    /// The driver's name in the reports (`DisplayProbe::driver`).
    driver_name: String,
    limit: DriverLimit,
    /// The last change of the display.
    display: Option<VirtualDisplayReport>,
    reports: HashMap<String, Entry>,
    /// Capture session → the size its Wayland host process offers.
    portal: HashMap<String, FrameSize>,
    /// The display is a physical monitor (a viewer's `request`).
    physical: bool,
    /// Give the physical monitor its own mode back.
    restore: bool,
    thread: bool,
    /// The sessions with a wish, for `SET_DISPLAY`.
    sessions: HashMap<String, SessionContext>,
    /// se set a mode the physical monitor lists (`physical_size`).
    physical_mode: bool,
    /// The sessions that asked for such a mode.
    physical_holders: HashSet<String>,
}

impl HostState {
    /// se changed a physical monitor and no session asks for it any more.
    fn give_back_due(&self) -> bool {
        self.physical
            && (self.planner.applied().is_some() || self.physical_mode)
            && !self.planner.has_claims()
            && self.physical_holders.is_empty()
    }
}

struct Shared {
    state: Mutex<HostState>,
    changed: Condvar,
    /// One change of the display at a time.
    turn: Mutex<()>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| Shared {
        state: Mutex::new(HostState {
            planner: Planner::default(),
            driver: None,
            driver_name: String::new(),
            limit: DriverLimit::NONE,
            display: None,
            reports: HashMap::new(),
            portal: HashMap::new(),
            physical: false,
            restore: false,
            thread: false,
            sessions: HashMap::new(),
            physical_mode: false,
            physical_holders: HashSet::new(),
        }),
        changed: Condvar::new(),
        turn: Mutex::new(()),
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn setup_report(
    progress: &SetupProgress,
    current: Option<FrameSize>,
    driver: &str,
) -> VirtualDisplayReport {
    match progress {
        SetupProgress::Running => {
            VirtualDisplayReport::new(VirtualDisplayState::Setup, current, driver)
        }
        SetupProgress::AfterRestart | SetupProgress::Done => {
            VirtualDisplayReport::new(VirtualDisplayState::AfterRestart, current, driver)
        }
        SetupProgress::Failed(reason) => {
            VirtualDisplayReport::new(VirtualDisplayState::Unsupported, current, driver)
                .with_reason(reason)
        }
    }
}

/// Starts the virtual display for a session (blocking: the caller runs it
/// beside the source preparation). Nothing without a wish, for a probe, a
/// preview or a metadata session, and on hosts without a driver. The answer
/// names `live`: the session may change its wish with `SET_DISPLAY`.
pub fn start(driver: Option<Arc<dyn VirtualDisplayDriver>>, start: &SessionStart) -> Started {
    let picture = matches!(start.mode, SessionMode::View | SessionMode::Control);
    let (Some(driver), Some(_), true, false) = (driver, start.wish, picture, start.metadata_only)
    else {
        return Started::default();
    };
    lock(&shared().state).sessions.insert(
        start.session.clone(),
        SessionContext {
            driver: driver.clone(),
            start: start.clone(),
            track: annotate::SourceTrack::opened(start.monitor.clone()),
            own_source: false,
        },
    );
    let claim = DisplayClaim {
        session: start.session.clone(),
        capture: start.capture.clone(),
    };
    let mut report = decide(&driver, start, false);
    report.live = true;
    Started {
        report: Some(report),
        claim: Some(claim),
    }
}

/// Records `report` as the session's own.
fn own(session: &str, report: VirtualDisplayReport) -> VirtualDisplayReport {
    lock(&shared().state)
        .reports
        .insert(session.to_string(), Entry::Own(report.clone()));
    report
}

/// The decision for one session with a wish: at its opening, and again
/// after `SET_DISPLAY` (`immediate`: the viewer's explicit choice waits for
/// no gap since the last change).
fn decide(
    driver: &Arc<dyn VirtualDisplayDriver>,
    start: &SessionStart,
    immediate: bool,
) -> VirtualDisplayReport {
    let wish = start.wish.unwrap_or_default();
    if wish == VirtualDisplayWish::Off && start.fixed.is_none() {
        let report = VirtualDisplayReport::new(VirtualDisplayState::Inactive, None, "")
            .with_reason(REASON_OFF);
        return own(&start.session, report);
    }
    let probe = match driver.probe() {
        Ok(probe) => probe,
        Err(error) => {
            let report = VirtualDisplayReport::new(VirtualDisplayState::Unsupported, None, "")
                .with_reason(&format!("probe_failed: {error}"));
            return own(&start.session, report);
        }
    };
    lock(&shared().state).physical = probe.physical_monitor;
    if let Some(size) = physical_size(wish, probe.physical_monitor, start.fixed) {
        return own(&start.session, physical(driver, start, &probe, size));
    }
    if let Some(reason) = probe
        .unsupported
        .as_deref()
        .filter(|_| !probe.setup_possible)
    {
        return own(&start.session, unsupported(&probe, reason));
    }
    if !needs_virtual(
        start.mode,
        start.metadata_only,
        start.wish,
        probe.physical_monitor,
    ) {
        let reason = if wish == VirtualDisplayWish::Off {
            REASON_OFF
        } else {
            REASON_PHYSICAL
        };
        let report =
            VirtualDisplayReport::new(VirtualDisplayState::Inactive, probe.current, &probe.driver)
                .with_reason(reason);
        return own(&start.session, report);
    }
    let fixed = start.fixed.is_some();
    let wanted = start.fixed.or(start.display_size).or(probe.current);
    if probe.portal {
        let size = wanted.and_then(|wanted| fit_mode(wanted, DriverLimit::NONE));
        if let Some(size) = size {
            lock(&shared().state)
                .portal
                .insert(start.capture.clone(), size);
        }
        let report = VirtualDisplayReport::new(VirtualDisplayState::Offered, size, &probe.driver);
        let report = annotate::with_reason(report, start.fixed, DriverLimit::NONE, true, false);
        return own(&start.session, report);
    }
    let beyond_limit = wanted
        .is_some_and(|wanted| fit_mode(wanted, probe.limit) != fit_mode(wanted, DriverLimit::NONE));
    let setup = (setup_allowed(start.wish, probe.physical_monitor)
        && start.may_setup
        && probe.setup_possible
        && (!probe.modes || beyond_limit))
        .then(|| driver.request_setup());
    if !probe.modes {
        let report = match &setup {
            Some(progress) => setup_report(progress, probe.current, &probe.driver),
            None => unsupported(
                &probe,
                probe.unsupported.as_deref().unwrap_or("no_display_modes"),
            ),
        };
        return own(&start.session, report);
    }
    let shared = shared();
    let due = {
        let mut state = lock(&shared.state);
        state.driver = Some(driver.clone());
        state.driver_name = probe.driver.clone();
        state.limit = probe.limit;
        state.restore = false;
        if let Some(size) = wanted {
            let control = start.mode == SessionMode::Control;
            state.planner.claim(&start.session, control, size, fixed);
        }
        state.reports.insert(start.session.clone(), Entry::Display);
        let now = if immediate { u64::MAX } else { now_ms() };
        state.planner.due(probe.limit, now)
    };
    let report = match due {
        Due::Now(size) => apply(shared, driver, size, &probe.driver),
        Due::Later(_) | Due::Idle => {
            let state = lock(&shared.state);
            state.display.clone().unwrap_or_else(|| {
                VirtualDisplayReport::new(
                    VirtualDisplayState::Active,
                    state.planner.applied().or(probe.current),
                    &probe.driver,
                )
            })
        }
    };
    let mut report = annotate::requested(report, start, probe.limit, matches!(due, Due::Later(_)));
    if let Some(progress) = &setup {
        report = setup_report(progress, report.size(), &probe.driver);
        lock(&shared.state)
            .reports
            .insert(start.session.clone(), Entry::Own(report.clone()));
    }
    ensure_thread(shared);
    shared.changed.notify_all();
    report
}

/// Applies `size` now (the caller saw it due) and records the result as
/// the display's report.
fn apply(
    shared: &'static Shared,
    driver: &Arc<dyn VirtualDisplayDriver>,
    size: FrameSize,
    name: &str,
) -> VirtualDisplayReport {
    lock(&shared.state).planner.changed(size, now_ms());
    let result = {
        let _turn = lock(&shared.turn);
        driver.apply(size)
    };
    let report = match result {
        Ok(actual) => VirtualDisplayReport::new(VirtualDisplayState::Active, Some(actual), name),
        Err(error) => {
            eprintln!("virtual display: {}x{}: {error}", size.width, size.height);
            VirtualDisplayReport::new(VirtualDisplayState::Unsupported, None, name)
                .with_reason("mode_refused")
        }
    };
    lock(&shared.state).display = Some(report.clone());
    report
}

/// A `SET_MODE` size of a session's full view (ignored without a claim or
/// with a fixed size); `true` when the session shows the display, decides
/// its size and the driver shows `size` as asked (`fit_mode`): the source
/// follows (`Pipeline::expect_source_size`).
pub fn resize(session: &str, size: Option<FrameSize>) -> bool {
    let Some(size) = size else {
        return false;
    };
    let Some(shared) = SHARED.get() else {
        return false;
    };
    let mut state = lock(&shared.state);
    let known = state.planner.resize(session, size);
    if known {
        shared.changed.notify_all();
    }
    let shows = state.sessions.get(session).is_some_and(|k| !k.own_source);
    known
        && shows
        && state.planner.owner_session() == Some(session)
        && state.planner.target(state.limit) == Some(size)
}

/// The session shows its own choice (`SET_MODE`) or the display (W1).
pub fn source_chosen(session: &str, own: bool) {
    let Some(shared) = SHARED.get() else {
        return;
    };
    if let Some(known) = lock(&shared.state).sessions.get_mut(session) {
        known.own_source = own;
    }
}

/// The size the host process of the capture session `capture` offers the
/// Wayland portal for its virtual source; `None` for any other session.
pub fn portal_size(capture: &str) -> Option<FrameSize> {
    let shared = SHARED.get()?;
    let state = lock(&shared.state);
    state.portal.get(capture).copied()
}

fn ensure_thread(shared: &'static Shared) {
    let mut state = lock(&shared.state);
    if state.thread {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("se-virtual-display".into())
        .spawn(move || follow(shared));
    match spawned {
        Ok(_) => state.thread = true,
        Err(error) => eprintln!("virtual display: no thread: {error}"),
    }
}

/// The background thread: applies the deciding session's size whenever it
/// is due, at most once per `MIN_RESIZE_GAP_MS`.
fn follow(shared: &'static Shared) {
    let mut state = lock(&shared.state);
    loop {
        if state.restore && !state.planner.has_claims() && state.physical_holders.is_empty() {
            state.restore = false;
            let driver = state.driver.clone();
            drop(state);
            if let Some(driver) = driver {
                let _turn = lock(&shared.turn);
                if let Err(error) = driver.restore() {
                    eprintln!("virtual display: the monitor's own mode: {error}");
                }
            }
            state = lock(&shared.state);
            state.planner.forget();
            state.display = None;
            state.physical_mode = false;
            continue;
        }
        let due = state.planner.due(state.limit, now_ms());
        match (due, state.driver.clone()) {
            (Due::Now(size), Some(driver)) => {
                let name = state.driver_name.clone();
                drop(state);
                apply(shared, &driver, size, &name);
                state = lock(&shared.state);
            }
            (Due::Later(wait), _) => {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(wait))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            _ => {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }
}
