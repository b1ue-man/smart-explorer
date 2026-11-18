//! Rule „Wahl gilt“ (user 2026-11-18), finding 3: a session the host moves
//! to se's virtual monitor names the monitor it shows; the viewer's own
//! choice of a source is no move.
use super::*;
use crate::screen::core::virtual_display::VirtualDisplayState;

fn active() -> VirtualDisplayReport {
    VirtualDisplayReport::new(
        VirtualDisplayState::Active,
        Some(FrameSize::new(2560, 1440)),
        "windows",
    )
}

#[test]
fn wahlgilt_task_source_track_tells_a_move_from_a_choice() {
    // Opening on DISPLAY1; the host shows its virtual DISPLAY3.
    let mut track = SourceTrack::opened(Some("DISPLAY1".into()));
    track.prepared(Some("DISPLAY3"));
    let key = track.annotate(active()).chain_key();
    assert_eq!(key, "active:2560x1440:windows;source=monitor:DISPLAY3");
    let report = track.annotate(active());
    assert_eq!(report.source_chosen.as_deref(), Some("monitor:DISPLAY1"));
    // The viewer chooses DISPLAY3 itself: no move any more.
    track.viewer_chooses();
    track.prepared(Some("DISPLAY3"));
    assert_eq!(track.annotate(active()), active());
    // `SET_DISPLAY` back to the physical monitor: the host picks DISPLAY1.
    track.display_changed();
    track.prepared(Some("DISPLAY1"));
    let report = track.annotate(active());
    assert_eq!(report.source.as_deref(), Some("monitor:DISPLAY1"));
    assert_eq!(report.source_chosen.as_deref(), Some("monitor:DISPLAY3"));
    // A host decision that keeps the chosen monitor is no move.
    let mut kept = SourceTrack::opened(Some("DP-1".into()));
    kept.prepared(Some("DP-1"));
    assert_eq!(kept.annotate(active()), active());
    // The opening's source preparation names the moved monitor.
    let start = include_str!("host_server_start.rs");
    assert!(start.contains("virtual_display_host::source_changed(&id, Some(monitor.clone()));"));
}
