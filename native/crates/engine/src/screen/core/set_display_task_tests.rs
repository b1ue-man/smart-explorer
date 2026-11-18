//! `SET_DISPLAY` (plan 2026-11-17 P9): the frame, its capability, the
//! monitor a session shows after a change, the fixed size at a physical
//! monitor and the host's decision within a running session.
use std::sync::{Arc, Mutex};

use super::*;
use crate::screen::core::virtual_display::{
    DriverLimit, VirtualDisplayChoice, VirtualDisplayReport, VirtualDisplayState,
    VirtualDisplayWish,
};
use crate::screen::core::virtual_display_host::{
    self, DisplayProbe, SessionStart, SetupProgress, VirtualDisplayDriver,
};
use crate::screen::core::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};
use crate::screen::core::wire_hello::SessionMode;

fn size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width, height)
}

fn monitor(id: &str, primary: bool, is_virtual: bool) -> MonitorInfo {
    MonitorInfo {
        id: id.into(),
        name: id.into(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        primary,
        is_virtual,
    }
}

#[test]
fn p9_task_set_display_frame_and_its_capability() {
    let choice = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Request,
        size: Some(size(2560, 1440)),
    };
    let (tag, payload) = encode_viewer_frame(&ViewerFrame::SetDisplay(choice)).unwrap();
    assert_eq!(tag, SET_DISPLAY);
    assert_eq!(tag, 54, "the tag after AUDIO (53)");
    assert!(payload.len() <= MAX_SET_DISPLAY_BYTES);
    assert_eq!(
        decode_viewer_frame(tag, &payload).unwrap(),
        ViewerFrame::SetDisplay(choice)
    );
    // Without a size the display follows the viewer.
    let follow = br#"{"wish":"off"}"#;
    assert_eq!(
        decode_viewer_frame(SET_DISPLAY, follow).unwrap(),
        ViewerFrame::SetDisplay(VirtualDisplayChoice {
            wish: VirtualDisplayWish::Off,
            size: None,
        })
    );
    // A side outside 1..=16384 and an oversized payload are refused.
    let wide = br#"{"wish":"request","size":{"width":20000,"height":1080}}"#;
    assert!(decode_viewer_frame(SET_DISPLAY, wide).is_err());
    let padded = format!(r#"{{"wish":"auto"{}}}"#, " ".repeat(MAX_SET_DISPLAY_BYTES));
    assert!(decode_viewer_frame(SET_DISPLAY, padded.as_bytes()).is_err());
    // `live` only in the answer of a host that takes the frame; absent in
    // the JSON of every other report, so older viewers see no change.
    let mut report =
        VirtualDisplayReport::new(VirtualDisplayState::Active, Some(size(1920, 1080)), "vdd");
    let plain = serde_json::to_value(&report).unwrap();
    assert!(plain.get("live").is_none());
    report.live = true;
    assert_eq!(serde_json::to_value(&report).unwrap()["live"], true);
    let old: VirtualDisplayReport =
        serde_json::from_str(r#"{"state":"active","width":1920,"height":1080}"#).unwrap();
    assert!(!old.live, "a host before P9");
    let mut hello: ScreenHelloOk = serde_json::from_value(serde_json::json!({
        "authorization": {"view": "allow", "control": "allow"},
        "platform": "windows",
        "approval": "not_required"
    }))
    .unwrap();
    assert!(!negotiated(None) && !negotiated(Some(&hello)));
    hello.virtual_display = Some(old);
    assert!(!negotiated(Some(&hello)), "an older host reopens");
    hello.virtual_display = Some(report);
    assert!(negotiated(Some(&hello)));
}

#[test]
fn p9_task_display_source_follows_what_the_display_shows() {
    // A Windows host: se's virtual monitor beside the physical one.
    let both = [
        monitor("DISPLAY1", false, false),
        monitor("DISPLAY3", true, true),
    ];
    let source = |current: Option<&str>, shown| display_source(&both, current, shown);
    let id = |text: &str| Some(SourceId::Monitor(text.into()));
    assert_eq!(source(Some("DISPLAY1"), Shown::Virtual), id("DISPLAY3"));
    assert_eq!(source(Some("DISPLAY3"), Shown::Virtual), id("DISPLAY3"));
    assert_eq!(source(Some("DISPLAY3"), Shown::Physical), id("DISPLAY1"));
    assert_eq!(source(Some("DISPLAY1"), Shown::Physical), id("DISPLAY1"));
    assert_eq!(source(None, Shown::Physical), id("DISPLAY1"));
    // A Linux host whose one output changes its mode: the session stays.
    let one = [monitor("DP-1", true, false)];
    assert_eq!(
        display_source(&one, Some("DP-1"), Shown::Virtual),
        id("DP-1")
    );
    // A monitor that went away: the primary physical one, then the first.
    let two = [monitor("DP-1", false, false), monitor("DP-2", true, false)];
    assert_eq!(
        display_source(&two, Some("gone"), Shown::Physical),
        id("DP-2")
    );
    let headless = [monitor("Virtual-1", true, true)];
    assert_eq!(
        display_source(&headless, None, Shown::Physical),
        id("Virtual-1")
    );
    assert_eq!(display_source(&[], None, Shown::Virtual), None);
}

#[test]
fn p9_task_physical_size_only_for_a_fixed_size_at_a_monitor() {
    use VirtualDisplayWish::{Auto, Off, Request};
    let fixed = Some(size(1920, 1080));
    assert_eq!(physical_size(Off, true, fixed), fixed);
    assert_eq!(physical_size(Auto, true, fixed), fixed);
    assert_eq!(physical_size(Request, true, fixed), None, "se's display");
    assert_eq!(physical_size(Off, false, fixed), None, "no monitor");
    assert_eq!(physical_size(Auto, false, fixed), None, "se's display");
    assert_eq!(physical_size(Off, true, None), None, "the window never");
}

/// A driver that records what the host asks of it.
struct Recording {
    probe: DisplayProbe,
    calls: Mutex<Vec<String>>,
}

impl Recording {
    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl VirtualDisplayDriver for Recording {
    fn probe(&self) -> std::io::Result<DisplayProbe> {
        Ok(self.probe.clone())
    }
    fn apply(&self, size: FrameSize) -> std::io::Result<FrameSize> {
        self.note(format!("apply {}x{}", size.width, size.height));
        Ok(size)
    }
    fn request_setup(&self) -> SetupProgress {
        SetupProgress::Failed("test".into())
    }
    fn restore(&self) -> std::io::Result<()> {
        self.note("restore".into());
        Ok(())
    }
    fn physical_mode(&self, size: FrameSize) -> std::io::Result<FrameSize> {
        self.note(format!("physical {}x{}", size.width, size.height));
        Ok(size)
    }
}

#[test]
fn p9_task_switch_decides_again_within_the_session() {
    let driver = Arc::new(Recording {
        probe: DisplayProbe {
            physical_monitor: true,
            modes: true,
            limit: DriverLimit::NONE,
            driver: "fake".into(),
            current: Some(size(3840, 2160)),
            unsupported: None,
            setup_possible: false,
            portal: false,
        },
        calls: Mutex::new(Vec::new()),
    });
    let start = SessionStart {
        session: "p9-switch-task".into(),
        capture: "p9-capture-1".into(),
        mode: SessionMode::View,
        metadata_only: false,
        wish: Some(VirtualDisplayWish::Request),
        fixed: Some(size(2560, 1440)),
        display_size: None,
        may_setup: false,
        monitor: Some("DISPLAY1".into()),
    };
    let dyn_driver: Arc<dyn VirtualDisplayDriver> = driver.clone();
    let started = virtual_display_host::start(Some(dyn_driver), &start);
    let report = started.report.clone().unwrap();
    assert!(report.live, "the opening names the capability");
    assert_eq!(report.chain_key(), "active:2560x1440:fake");
    let choice = |wish, size| VirtualDisplayChoice { wish, size };
    // A new size at once, though less than a second passed.
    let switched = virtual_display_host::switch(
        "p9-switch-task",
        choice(VirtualDisplayWish::Request, Some(size(1920, 1080))),
        "p9-capture-2",
    )
    .unwrap();
    assert_eq!(switched.report.chain_key(), "active:1920x1080:fake");
    assert!(switched.report.live);
    assert_eq!(switched.shown(), Shown::Virtual);
    assert_eq!(switched.monitor.as_deref(), Some("DISPLAY1"));
    // The physical monitor in a mode it lists.
    let physical = virtual_display_host::switch(
        "p9-switch-task",
        choice(VirtualDisplayWish::Off, Some(size(1280, 720))),
        "p9-capture-3",
    )
    .unwrap();
    assert_eq!(physical.report.chain_key(), "inactive:off");
    assert_eq!(physical.report.size(), Some(size(1280, 720)));
    assert_eq!(physical.shown(), Shown::Physical);
    // The monitor as it is: its own mode back at once.
    let off = virtual_display_host::switch(
        "p9-switch-task",
        choice(VirtualDisplayWish::Off, None),
        "p9-capture-4",
    )
    .unwrap();
    assert_eq!(off.report.chain_key(), "inactive:off");
    assert_eq!(
        *driver.calls.lock().unwrap(),
        [
            "apply 2560x1440",
            "apply 1920x1080",
            "physical 1280x720",
            "restore"
        ]
    );
    assert!(virtual_display_host::switch(
        "p9-no-such-session",
        choice(VirtualDisplayWish::Auto, None),
        "p9-capture-5",
    )
    .is_none());
    drop(started);
}
