//! Task tests of E48 (L5): the end reason `session_changed` on the wire, an
//! unknown reason stays readable, and after `session_changed` the relay
//! dials once per attempt, at most once per second, for at most 30 s
//! (simulated clock), tries on while the host's new session is not ready,
//! and ends with any other refusal's own code.
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::{
    again, changed_session, Clock, Dial, ModeMemory, Outcome, Pending, Schedule, Step,
    RECONNECT_EVERY, RECONNECT_FOR,
};
use crate::screen::core::frame_size_caps::FrameSize;
use crate::screen::core::session_state::{
    is_session_changed, session_changed_error, EndReason, SessionState,
};
use crate::screen::core::viewer_client::ViewerError;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire::{from_json, to_json};
use crate::screen::core::wire_control::{
    HostFrame, Ping, SetMode, StateReport, StreamMode, ViewerFrame,
};

/// A clock whose sleep moves its time on at once.
#[derive(Clone)]
struct SimulatedClock(Arc<Mutex<Instant>>);

impl Clock for SimulatedClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
    fn sleep(&self, time: Duration) -> Pending<()> {
        *self.0.lock().unwrap() += time;
        Box::pin(std::future::ready(()))
    }
}

/// A dialer that answers from a script (then „not reached“) and notes the
/// time of every dial.
struct ScriptedDial {
    clock: SimulatedClock,
    dials: Arc<Mutex<Vec<Instant>>>,
    answers: Mutex<VecDeque<Result<u32, ViewerError>>>,
}

impl Dial for ScriptedDial {
    type Session = u32;
    fn dial(&self, _remaining: Duration) -> Pending<Result<u32, ViewerError>> {
        self.dials.lock().unwrap().push(self.clock.now());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(unreachable_host()));
        Box::pin(std::future::ready(answer))
    }
}

fn unreachable_host() -> ViewerError {
    ViewerError::Io(io::Error::other("not reached"))
}

fn run(
    answers: Vec<Result<u32, ViewerError>>,
    input: &mut mpsc::Receiver<Option<super::RelayedFrame>>,
    check: impl FnOnce(Outcome<u32>, Vec<Instant>, Instant),
) {
    let start = Instant::now();
    let clock = SimulatedClock(Arc::new(Mutex::new(start)));
    let dials = Arc::new(Mutex::new(Vec::new()));
    let dialer = ScriptedDial {
        clock: clock.clone(),
        dials: dials.clone(),
        answers: Mutex::new(answers.into()),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut schedule = Schedule::new(start);
    let mut modes = ModeMemory::default();
    let outcome = runtime.block_on(again(dialer, clock, &mut schedule, input, &mut modes));
    let dials = dials.lock().unwrap().clone();
    check(outcome, dials, start);
}

#[test]
fn bildschirm_sitzung_task_session_changed_kodiert_dekodiert_und_unbekannt_lesbar() {
    assert_eq!(EndReason::SessionChanged.key(), "session_changed");
    for reason in EndReason::ALL {
        assert_eq!(EndReason::from_key(reason.key()), Some(reason));
    }
    let report = StateReport {
        state: SessionState::Ended,
        reason: Some(EndReason::SessionChanged.key().into()),
        stream: None,
    };
    let decoded: StateReport = from_json(&to_json(&report).unwrap()).unwrap();
    assert_eq!(decoded, report);
    assert!(changed_session(&HostFrame::State(decoded)));
    // A reason of a newer host: still a readable report, no known reason.
    let newer: StateReport =
        from_json(br#"{"state":"ended","reason":"reason_of_a_newer_host"}"#).unwrap();
    assert_eq!(newer.reason.as_deref(), Some("reason_of_a_newer_host"));
    assert_eq!(EndReason::from_key("reason_of_a_newer_host"), None);
    assert!(!changed_session(&HostFrame::State(newer)));
    let failed = StateReport {
        state: SessionState::Ended,
        reason: Some(EndReason::Failed.key().into()),
        stream: None,
    };
    assert!(!changed_session(&HostFrame::State(failed)));
    assert!(is_session_changed(&session_changed_error()));
    assert!(!is_session_changed(&io::Error::other(
        "the active graphical session changed"
    )));
}

#[test]
fn bildschirm_sitzung_task_plan_einmal_je_sekunde_hoechstens_30_s() {
    let start = Instant::now();
    let mut schedule = Schedule::new(start);
    assert_eq!(
        schedule.step(start),
        Step::Attempt {
            remaining: RECONNECT_FOR
        }
    );
    let half = Duration::from_millis(500);
    assert_eq!(schedule.step(start + half), Step::Wait(half));
    assert!(matches!(
        schedule.step(start + RECONNECT_EVERY),
        Step::Attempt { .. }
    ));
    assert_eq!(schedule.step(start + RECONNECT_FOR), Step::GiveUp);
}

#[test]
fn bildschirm_sitzung_task_wiederverbindung_ein_handshake_je_versuch_ende_nach_30_s() {
    let (_keep, mut input) = mpsc::channel(4);
    run(Vec::new(), &mut input, |outcome, dials, start| {
        assert!(matches!(outcome, Outcome::GaveUp));
        assert_eq!(dials.len(), 30, "one dial per second for 30 s");
        assert_eq!(dials[0], start, "the first attempt follows at once");
        for pair in dials.windows(2) {
            assert!(pair[1] - pair[0] >= RECONNECT_EVERY);
        }
        assert!(*dials.last().unwrap() - start < RECONNECT_FOR);
    });
}

#[test]
fn bildschirm_sitzung_task_wiederverbindung_oeffnet_oder_endet_mit_ablehnung() {
    let (_keep, mut input) = mpsc::channel(4);
    // Not reached, then a new session that is not ready yet: tried on.
    let opened = vec![
        Err(unreachable_host()),
        Err(ViewerError::Refused("no_display".into())),
        Err(ViewerError::Refused("indicator_unavailable".into())),
        Ok(7),
    ];
    run(opened, &mut input, |outcome, dials, _| {
        assert!(matches!(outcome, Outcome::Opened(7)));
        assert_eq!(dials.len(), 4);
    });
    let refused = vec![
        Err(unreachable_host()),
        Err(ViewerError::Refused("permission_denied".into())),
    ];
    run(refused, &mut input, |outcome, dials, _| {
        match outcome {
            Outcome::Refused(error) => assert_eq!(error.code(), "permission_denied"),
            _ => panic!("a refused reconnection ends with the refusal"),
        }
        assert_eq!(dials.len(), 2, "no attempt after the refusal");
    });
    let (closing, mut closed) = mpsc::channel(4);
    closing.try_send(None).unwrap();
    run(Vec::new(), &mut closed, |outcome, dials, _| {
        assert!(matches!(outcome, Outcome::ViewerClosed));
        assert_eq!(dials.len(), 1, "the viewer closed during the first attempt");
    });
}

fn mode(
    mode: StreamMode,
    source: Option<SourceId>,
    size: Option<(u32, u32)>,
    fps: Option<u16>,
) -> ViewerFrame {
    ViewerFrame::SetMode(SetMode {
        mode,
        source,
        size: size.map(|(width, height)| FrameSize::new(width, height)),
        fps,
        sharpness: false,
        lossless: false,
    })
}

fn replayed(memory: &ModeMemory) -> Vec<SetMode> {
    memory.replay(memory.source())
}

#[test]
fn verbindungen_task_relay_replays_the_set_mode_choices_after_session_changed() {
    // A viewer that never sent `SET_MODE` leaves the new session as its
    // first request made it.
    let mut memory = ModeMemory::default();
    memory.note(&ViewerFrame::Ping(Ping {
        sequence: 1,
        sent_us: 0,
    }));
    assert!(replayed(&memory).is_empty());

    // The group view: a tile (preview, own size), raised to the full view of
    // another monitor with the display size, then a size change that names
    // no source.
    let monitor = SourceId::Monitor("DP-2".into());
    memory.note(&mode(StreamMode::Preview, None, Some((320, 180)), Some(2)));
    memory.note(&mode(
        StreamMode::Full,
        Some(monitor.clone()),
        Some((1920, 1080)),
        Some(30),
    ));
    memory.note(&mode(StreamMode::Full, None, Some((1280, 720)), None));
    let frames = replayed(&memory);
    assert_eq!(frames.len(), 2, "the preview choice and the full view");
    // The source travels once, first; the stream kind in force comes last.
    assert_eq!(frames[0].mode, StreamMode::Preview);
    assert_eq!(frames[0].source, Some(monitor));
    assert_eq!(frames[0].size, Some(FrameSize::new(320, 180)));
    assert_eq!(frames[0].fps, Some(2));
    assert_eq!(frames[1].mode, StreamMode::Full);
    assert_eq!(frames[1].source, None);
    assert_eq!(
        frames[1].size,
        Some(FrameSize::new(1280, 720)),
        "the later size wins"
    );
    assert_eq!(frames[1].fps, Some(30), "an omitted frame rate is retained");

    // Back to the tile: the last kind is the preview, the full view keeps
    // its size, and what the last frame set (sharpness) is the one replayed.
    memory.note(&ViewerFrame::SetMode(SetMode {
        mode: StreamMode::Preview,
        source: None,
        size: None,
        fps: None,
        sharpness: true,
        lossless: false,
    }));
    let frames = replayed(&memory);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].mode, StreamMode::Full);
    assert_eq!(frames[0].size, Some(FrameSize::new(1280, 720)));
    assert!(frames[0].source.is_some());
    assert_eq!(frames[1].mode, StreamMode::Preview);
    assert_eq!(frames[1].size, Some(FrameSize::new(320, 180)));
    assert!(frames.iter().all(|frame| frame.sharpness));

    // One kind only: one frame, with the source.
    let mut single = ModeMemory::default();
    single.note(&mode(
        StreamMode::Full,
        Some(SourceId::Window("42".into())),
        None,
        None,
    ));
    let frames = replayed(&single);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].mode, StreamMode::Full);
    assert_eq!(frames[0].source, Some(SourceId::Window("42".into())));
}

#[test]
fn review1119_task_relay_keeps_the_choice_under_a_marked_fallback() {
    use super::RelayedFrame;
    let lossless = |on: bool| {
        let ViewerFrame::SetMode(mut set) = mode(StreamMode::Full, None, None, None) else {
            unreachable!("mode() builds a SET_MODE");
        };
        set.lossless = on;
        ViewerFrame::SetMode(set)
    };
    let relayed = |frame, fallback| RelayedFrame { frame, fallback };
    // The automatic fallback to the tiles (round 11, N2): the host gets it,
    // the rebuilt session opens with the viewer's own choice.
    let mut memory = ModeMemory::default();
    memory.note_relayed(&relayed(lossless(false), false));
    memory.note_relayed(&relayed(lossless(true), true));
    let frames = replayed(&memory);
    assert_eq!(frames.len(), 1);
    assert!(!frames[0].lossless, "the fallback is no choice");
    // A fallback before any choice keeps the default, not the tiles.
    let mut first = ModeMemory::default();
    first.note_relayed(&relayed(lossless(true), true));
    assert!(!replayed(&first)[0].lossless);
    // „Verlustfrei“ chosen by the user stays chosen.
    let mut chosen = ModeMemory::default();
    chosen.note_relayed(&relayed(lossless(true), false));
    chosen.note_relayed(&relayed(lossless(true), true));
    assert!(replayed(&chosen)[0].lossless);
}

fn first_request(source: Option<SourceId>) -> crate::screen::core::viewer_client::ViewerRequest {
    crate::screen::core::viewer_client::ViewerRequest {
        mode: crate::screen::core::wire_hello::SessionMode::View,
        source,
        decoders: Vec::new(),
        display_size: None,
        max_fps: 30,
        frame_capabilities: None,
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
        codec_choice: None,
        max_rate_bps: None,
        delivery: None,
    }
}

/// The sources a replay asks for, in order (`SET_MODE`, `SET_DISPLAY`).
fn source_changes(frames: &[ViewerFrame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ViewerFrame::SetMode(set) => set.source.as_ref().map(|s| s.to_string()),
            ViewerFrame::SetDisplay(_) => Some("display".into()),
            _ => None,
        })
        .collect()
}

/// Round 12 (H3, H4): a session built again opens with the display in its
/// hello at a monitor and reaches a window chosen after it with one source
/// change (before: window, display, window); a display chosen after the
/// source is not sent twice; a source the new session lacks falls back to
/// the host's own one and is asked for again by `SET_MODE`.
#[test]
fn r12_task_rebuild_opens_with_the_display_and_one_source_change() {
    use super::rebuild;
    use crate::screen::core::virtual_display::{VirtualDisplayChoice, VirtualDisplayWish};
    let display = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Request,
        size: Some(FrameSize::new(2560, 1440)),
    };
    let window = SourceId::Window("42".into());
    let first = first_request(Some(SourceId::Monitor("DP-1".into())));

    // Display first, then a window: the hello carries the display at the
    // host's own monitor; one `SET_MODE` reaches the window.
    let mut memory = ModeMemory::default();
    memory.note(&ViewerFrame::SetDisplay(display));
    memory.note(&mode(StreamMode::Full, Some(window.clone()), None, None));
    let request = rebuild::request(&memory, &first, None);
    assert_eq!(request.virtual_display, Some(display));
    assert_eq!(request.source, None, "the display decides at a monitor");
    let frames = rebuild::frames(&memory, &first, &request, false, false);
    assert_eq!(source_changes(&frames), vec![window.to_string()]);

    // A window, then the display: the session ends at the display's monitor
    // with no source change at all.
    let mut memory = ModeMemory::default();
    memory.note(&mode(StreamMode::Full, Some(window.clone()), None, None));
    memory.note(&ViewerFrame::SetDisplay(display));
    let request = rebuild::request(&memory, &first, None);
    assert_eq!(request.virtual_display, Some(display));
    assert_eq!(request.source, None);
    let frames = rebuild::frames(&memory, &first, &request, false, false);
    assert!(source_changes(&frames).is_empty(), "{frames:?}");

    // A chosen monitor stays in the hello with the display.
    let monitor = SourceId::Monitor("DP-2".into());
    let mut memory = ModeMemory::default();
    memory.note(&mode(StreamMode::Full, Some(monitor.clone()), None, None));
    memory.note(&ViewerFrame::SetDisplay(display));
    assert_eq!(
        rebuild::request(&memory, &first, None).source,
        Some(monitor)
    );

    // A first opening tried again opened with the first request: the
    // display chosen meanwhile still goes, after the source (Codex 11/4).
    let frames = rebuild::frames(&memory, &first, &first, false, false);
    assert_eq!(
        source_changes(&frames),
        vec!["monitor:DP-2".to_string(), "display".to_string()]
    );

    // No display: the source in force opens the session, `SET_MODE` names
    // it again (no change on the host, `host_visibility.rs`).
    let mut memory = ModeMemory::default();
    memory.note(&mode(StreamMode::Full, Some(window.clone()), None, None));
    let request = rebuild::request(&memory, &first, Some(650_000));
    assert_eq!(request.source, Some(window.clone()));
    assert_eq!(request.max_rate_bps, Some(650_000));
    // H4: the window is gone in the new session: the fallback opens on the
    // host's own source and the replay asks for the window again.
    let fallback = rebuild::without_source(&request).expect("a source to leave");
    assert_eq!(fallback.source, None);
    let frames = rebuild::frames(&memory, &first, &fallback, false, false);
    assert_eq!(source_changes(&frames), vec![window.to_string()]);
    assert!(rebuild::without_source(&fallback).is_none());
}

/// Round 13 (H3a): only a refusal of the source itself (`no_display` with a
/// reason that names the source gone) lets the next attempt open on the
/// host's own source; a bare `no_display` (no graphical session yet) or a
/// transient reason keeps the chosen one.
#[test]
fn r13_task_rebuild_falls_back_only_for_a_refused_source() {
    use super::rebuild::source_refused;
    let refused = |text: &str| source_refused(&ViewerError::Refused(text.into()));
    assert!(refused("no_display: window_not_found"));
    assert!(refused("no_display: monitor_not_found"));
    assert!(refused("no_display: source_permission_revoked"));
    assert!(!refused("no_display"));
    assert!(!refused("no_display:  "));
    assert!(!refused("no_display: session_inactive"));
    assert!(!refused("no_display: host_process: connection reset"));
    assert!(!refused("no_display: capture_unavailable"));
    assert!(refused("no_display: x11: X11 error")); // round 14, B1
    assert!(refused("no_display: portal_selects_source"));
    assert!(refused("no_display: stale_or_foreign_family_selection"));
    assert!(!refused("capture_failed: window_not_found"));
    assert!(!source_refused(&ViewerError::Unsupported));
}

/// Round 13 (H4, H3b): a monitor named only in the first request stays the
/// session's source when a display is chosen later (before, the hello lost
/// it and „Auflösung“ went to the host's own monitor); a fallback that
/// opened elsewhere asks for that monitor again and then for the display,
/// and the viewer's settings follow without the source, so a refused source
/// keeps them. se's own display shows at whatever monitor: no source change.
#[test]
fn r13_task_rebuild_keeps_the_first_monitor_and_asks_for_it_again() {
    use super::rebuild;
    use crate::screen::core::virtual_display::{VirtualDisplayChoice, VirtualDisplayWish};
    let fixed = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Auto,
        size: Some(FrameSize::new(1920, 1080)),
    };
    let monitor = SourceId::Monitor("DP-2".into());
    let first = first_request(Some(monitor.clone()));
    let mut memory = ModeMemory::default();
    memory.note(&mode(StreamMode::Full, None, Some((1280, 720)), None));
    memory.note(&ViewerFrame::SetDisplay(fixed));
    let request = rebuild::request(&memory, &first, None);
    assert_eq!(request.source, Some(monitor.clone()));
    assert_eq!(request.virtual_display, Some(fixed));
    let frames = rebuild::frames(&memory, &first, &request, false, false);
    assert!(source_changes(&frames).is_empty(), "{frames:?}");

    let fallback = rebuild::without_source(&request).expect("a source to leave");
    let frames = rebuild::frames(&memory, &first, &fallback, false, false);
    assert_eq!(
        source_changes(&frames),
        vec![monitor.to_string(), "display".to_string()]
    );
    let sets: Vec<&SetMode> = frames
        .iter()
        .filter_map(|frame| match frame {
            ViewerFrame::SetMode(set) => Some(set),
            _ => None,
        })
        .collect();
    assert_eq!(sets.len(), 2, "{frames:?}");
    assert_eq!(sets[1].source, None);
    assert_eq!(sets[1].size, sets[0].size);
    assert_eq!(sets[1].size, Some(FrameSize::new(1280, 720)));

    // Without a `SET_MODE` of the viewer no frame carries the monitor:
    // neither it nor the display is asked again (no needless display change).
    let mut memory = ModeMemory::default();
    memory.note(&ViewerFrame::SetDisplay(fixed));
    let request = rebuild::request(&memory, &first, None);
    let fallback = rebuild::without_source(&request).expect("a source to leave");
    let frames = rebuild::frames(&memory, &first, &fallback, false, false);
    assert!(source_changes(&frames).is_empty(), "{frames:?}");

    let own = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Request,
        size: None,
    };
    let mut memory = ModeMemory::default();
    memory.note(&mode(StreamMode::Full, Some(monitor.clone()), None, None));
    memory.note(&ViewerFrame::SetDisplay(own));
    let first = first_request(None);
    let request = rebuild::request(&memory, &first, None);
    let fallback = rebuild::without_source(&request).expect("a source to leave");
    let frames = rebuild::frames(&memory, &first, &fallback, false, false);
    assert!(source_changes(&frames).is_empty(), "{frames:?}");
}
