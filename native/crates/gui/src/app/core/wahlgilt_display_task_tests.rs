//! Findings 10 and 12 of the user rule of 2026-11-18: a host without a
//! report on „Monitor“/„Auflösung“ is named as not supporting them, and a
//! session the host moved to another monitor names the source it really
//! shows („Quelle: <gewählt> → jetzt <tatsächlich>“), never the raw
//! `;source=` key.
use super::state_text;
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display::{VirtualDisplayReport, VirtualDisplayState};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::HostStats;
use crate::share::access_grant::CapabilityDecision;

#[test]
fn wahlgilt_task_display_line_names_what_the_host_did() {
    let shared = super::super::super::screen_session::Shared::new(false, 0);
    // Before the host's answer nothing is claimed.
    assert_eq!(state_text(&shared.info().clone(), None), None);
    // A host before P5 sends no report: the wish is not taken.
    let hello = super::super::super::screen_viewer_fake::hello(CapabilityDecision::Allow);
    shared.update(|info| info.hello = Some(hello.clone()));
    assert_eq!(
        state_text(&shared.info().clone(), None).as_deref(),
        Some("Monitor und Auflösung: vom Gerät nicht unterstützt")
    );
    // The host moved the session to its own monitor.
    let size = FrameSize::new(1920, 1080);
    let moved = VirtualDisplayReport::new(VirtualDisplayState::Active, Some(size), "windows")
        .with_source(Some("monitor:1"), "monitor:7");
    let mut with_report = hello.clone();
    with_report.virtual_display = Some(moved.clone());
    shared.update(|info| info.hello = Some(with_report.clone()));
    let chosen = SourceId::Monitor("1".into());
    let line = state_text(&shared.info().clone(), Some(&chosen)).unwrap();
    assert!(
        line.ends_with(" · Quelle: Monitor 1 → jetzt Monitor 7"),
        "{line}"
    );
    assert_eq!(line.matches("Quelle").count(), 1, "{line}");
    // The statistics name the source now; the raw key never shows.
    let stats = HostStats {
        chain: Some(ChainReport {
            virtual_display: Some("active:2560x1440:windows;source=monitor:8".into()),
            ..ChainReport::default()
        }),
        ..HostStats::default()
    };
    shared.update(|info| info.stats = Some(stats));
    let line = state_text(&shared.info().clone(), Some(&chosen)).unwrap();
    assert!(line.contains(" · jetzt active:2560x1440:windows"), "{line}");
    assert!(!line.contains(";source="), "{line}");
    assert!(
        line.ends_with(" · Quelle: Monitor 1 → jetzt Monitor 8"),
        "{line}"
    );
    // Unmoved: no source part.
    let off = VirtualDisplayReport::new(VirtualDisplayState::Inactive, None, "randr")
        .with_reason("physical_monitor");
    with_report.virtual_display = Some(off.clone());
    shared.update(|info| {
        info.hello = Some(with_report.clone());
        info.stats = None;
    });
    assert_eq!(
        state_text(&shared.info().clone(), Some(&chosen)),
        Some(off.text())
    );
    // A setup or why not (the driver installed at use time) in German.
    let stats = HostStats {
        chain: Some(ChainReport {
            virtual_display: Some("unsupported:windows_display_driver_declined".into()),
            ..ChainReport::default()
        }),
        ..HostStats::default()
    };
    shared.update(|info| info.stats = Some(stats));
    let line = state_text(&shared.info().clone(), Some(&chosen)).unwrap();
    assert!(
        line.ends_with(" · jetzt nicht möglich (die Administrator-Bestätigung für den Treiber wurde auf dem Gastgeber abgelehnt; eine erneute Wahl fragt wieder)"),
        "{line}"
    );
}
