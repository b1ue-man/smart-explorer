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
    input: &mut mpsc::Receiver<Option<ViewerFrame>>,
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
    let outcome = runtime.block_on(again(dialer, clock, &mut schedule, input));
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
    memory.replay()
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
