//! Rule „Wahl gilt“ (user 2026-11-18) for „Monitor“ and „Auflösung“: a
//! wish at a window source is answered, a fixed size the host could not set
//! exactly names what was chosen, what was set and why, a refused physical
//! mode says so, and every text names the size it knows.
use super::*;
use crate::screen::core::wire_hello::SessionMode;

fn size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width, height)
}

#[test]
fn wahlgilt_task_window_source_answers_the_display_wish() {
    use VirtualDisplayWish::{Auto, Off, Request};
    let fixed = Some(size(2560, 1440));
    let view = SessionMode::View;
    // `request` or a fixed size cannot apply to a window: Unsupported.
    for (wish, fixed) in [(Request, None), (Auto, fixed), (Off, fixed)] {
        let report = VirtualDisplayReport::not_monitor(Some(wish), fixed, view, false).unwrap();
        assert_eq!(report.state, VirtualDisplayState::Unsupported, "{wish:?}");
        assert_eq!(report.reason.as_deref(), Some(REASON_SOURCE_NOT_MONITOR));
        assert_eq!(
            report.limit().as_deref(),
            Some("virtual_display_unsupported:source_not_monitor")
        );
        assert_eq!(report.requested, fixed);
        assert!(report
            .text()
            .contains("die gewählte Quelle ist kein Monitor"));
        assert!(report.text().contains("die gewählte Quelle hat Vorrang"));
    }
    // `auto` decides nothing for a window, `off` changes nothing: reported.
    let auto = VirtualDisplayReport::not_monitor(Some(Auto), None, view, false).unwrap();
    assert_eq!(auto.state, VirtualDisplayState::Inactive);
    assert_eq!(auto.chain_key(), "inactive:source_not_monitor");
    let off = VirtualDisplayReport::not_monitor(Some(Off), None, view, false).unwrap();
    assert_eq!(off.chain_key(), "inactive:off");
    // No wish, a metadata session, a probe: no answer, as before.
    assert!(VirtualDisplayReport::not_monitor(None, fixed, view, false).is_none());
    assert!(VirtualDisplayReport::not_monitor(Some(Request), None, view, true).is_none());
    let probe = SessionMode::Probe;
    assert!(VirtualDisplayReport::not_monitor(Some(Request), None, probe, false).is_none());
    let text = include_str!("host_server_start.rs");
    assert!(text.contains("VirtualDisplayReport::not_monitor("));
}

#[test]
fn wahlgilt_task_fixed_size_names_chosen_set_and_reason() {
    // se's mode rules: the width a multiple of 8, at least 640 × 360.
    assert_eq!(fit_reason(size(1920, 1080), DriverLimit::NONE), None);
    assert_eq!(
        fit_reason(size(1366, 768), DriverLimit::NONE),
        Some(ADJUST_MODE_RULES)
    );
    assert_eq!(
        fit_reason(size(320, 200), DriverLimit::NONE),
        Some(ADJUST_MODE_RULES)
    );
    // The driver's limit alone.
    let small = DriverLimit::sides(1920, 1080);
    assert_eq!(
        fit_reason(size(2560, 1440), small),
        Some(ADJUST_DRIVER_LIMIT)
    );
    // Why the display shows another size than the fixed one.
    let fixed = size(2560, 1440);
    let fitted = fit_mode(fixed, small);
    assert_eq!(adjust_reason(fixed, Some(fixed), small, true, false), None);
    assert_eq!(
        adjust_reason(fixed, fitted, small, true, false),
        Some(ADJUST_DRIVER_LIMIT)
    );
    let other = Some(size(1280, 720));
    assert_eq!(
        adjust_reason(fixed, other, DriverLimit::NONE, false, false),
        Some(ADJUST_OTHER_SESSION)
    );
    assert_eq!(
        adjust_reason(fixed, other, DriverLimit::NONE, true, true),
        Some(ADJUST_PENDING)
    );
    assert_eq!(
        adjust_reason(fixed, None, DriverLimit::NONE, true, false),
        Some(ADJUST_DRIVER)
    );
    // The report: chosen, set, reason; an equal size adds nothing.
    let active = VirtualDisplayReport::new(VirtualDisplayState::Active, fitted, "randr");
    assert_eq!(active.clone().with_requested(size(1920, 1080), "x"), active);
    let report = active.with_requested(fixed, ADJUST_DRIVER_LIMIT);
    assert_eq!(report.requested, Some(fixed));
    assert_eq!(
        report.chain_key(),
        "active:1920x1080:randr;requested=2560x1440;adjusted=driver_limit"
    );
    assert_eq!(
        report.text(),
        "Virtueller Bildschirm: 1920×1080 (randr) · gewählt 2560×1440, eingestellt 1920×1080, \
Grund: Grenze des Anzeigetreibers"
    );
    // The nearest mode of a physical monitor and a refused one.
    let nearest = VirtualDisplayReport::new(
        VirtualDisplayState::Inactive,
        Some(size(2560, 1600)),
        "randr",
    )
    .with_reason(REASON_OFF)
    .with_requested(fixed, ADJUST_PHYSICAL_MODE);
    assert!(nearest
        .text()
        .contains("aus (physischer Monitor gewählt, 2560×1600)"));
    assert!(nearest.text().contains("sein nächstliegender Modus gilt"));
    let refused = VirtualDisplayReport::new(
        VirtualDisplayState::Inactive,
        Some(size(3840, 2160)),
        "randr",
    )
    .with_reason(REASON_PHYSICAL_MODE_REFUSED)
    .with_requested(fixed, &format!("{ADJUST_REFUSED}: BadMatch"));
    let text = refused.text();
    assert!(text.contains("der Monitor lehnte die gewählte Auflösung ab, 3840×2160"));
    assert!(text.contains("Grund: der Monitor lehnte den Modus ab: BadMatch"));
    assert_eq!(
        refused.chain_key(),
        "inactive:physical_mode_refused;requested=2560x1440;adjusted=refused"
    );
    // `Inactive` names the size it knows.
    let monitor = VirtualDisplayReport::new(
        VirtualDisplayState::Inactive,
        Some(size(1920, 1080)),
        "randr",
    )
    .with_reason(REASON_PHYSICAL);
    assert_eq!(
        monitor.text(),
        "Virtueller Bildschirm: aus (Monitor am Gerät, 1920×1080)"
    );
    // Older JSON reads; the new fields stay out of a report without them.
    let old: VirtualDisplayReport =
        serde_json::from_str(r#"{"state":"active","width":1920,"height":1080}"#).unwrap();
    assert_eq!(old.requested, None);
    assert_eq!(old.source, None);
    let plain = serde_json::to_value(&old).unwrap();
    for field in ["requested", "adjusted", "source", "source_chosen"] {
        assert!(plain.get(field).is_none(), "{field}");
    }
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["requested"]["width"], 2560);
    assert_eq!(json["adjusted"], "driver_limit");
}

#[test]
fn wahlgilt_task_moved_source_is_named_to_the_viewer() {
    let active = VirtualDisplayReport::new(
        VirtualDisplayState::Active,
        Some(size(2560, 1440)),
        "windows",
    );
    // The same monitor adds nothing.
    let same = active
        .clone()
        .with_source(Some("monitor:DISPLAY1"), "monitor:DISPLAY1");
    assert_eq!(same, active);
    let moved = active.with_source(Some("monitor:DISPLAY1"), "monitor:DISPLAY3");
    assert_eq!(moved.source.as_deref(), Some("monitor:DISPLAY3"));
    assert_eq!(moved.source_chosen.as_deref(), Some("monitor:DISPLAY1"));
    assert_eq!(
        moved.chain_key(),
        "active:2560x1440:windows;source=monitor:DISPLAY3"
    );
    assert!(moved
        .text()
        .ends_with("· Quelle: monitor:DISPLAY1 → jetzt monitor:DISPLAY3 (der virtuelle Bildschirm ist ein eigener Monitor)"));
    let back: VirtualDisplayReport =
        serde_json::from_value(serde_json::to_value(&moved).unwrap()).unwrap();
    assert_eq!(back, moved);
}
