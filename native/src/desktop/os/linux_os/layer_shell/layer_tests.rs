//! DSK-M7 checks that need no compositor: the output keys, the key table,
//! the scale arithmetic, the pointer mapping, the host's global point and the
//! namespace by desktop. The checks with a compositor are in
//! `layer_task_tests.rs`, those of the clipboard formats in
//! `layer_clipboard_tests.rs`.
use eframe::egui::{self, Event, Key, Modifiers, MouseWheelUnit, PointerButton};
use smithay_client_toolkit::seat::pointer::CursorIcon;

use super::host::LayerHost;
use super::input::cursor_for;
use super::keysyms::{is_typed_text, key_events, keysym_to_egui, modifiers_of, KeyPress};
use super::pointer::{button_of, wheel_delta};
use super::registry::{build_outputs, output_key, OutputScale, RawOutput};
use crate::app::desktop_surface_host::{self as host_ui, NativeWindow, SurfaceHost};
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::layout::DesktopLayout;
use crate::desktop::core::mechanism::{
    self, DisplayServer, LayerKeyboard, LayerNamespace, Mechanism, SessionFacts,
};
use crate::desktop::os::Monitor;

fn raw(
    global_name: u32,
    name: Option<&str>,
    make: &str,
    model: &str,
    origin: [i32; 2],
) -> RawOutput {
    RawOutput {
        global_name,
        name: name.map(str::to_string),
        xdg_name: None,
        make: make.into(),
        model: model.into(),
        origin,
        size: [1920, 1080],
        scale_factor: 1,
    }
}

#[test]
fn desktop_task_layer_output_keys() {
    // (1) the connector name, (2) the xdg-output name, (3) make and model
    // with everything outside [A-Za-z0-9-._] as `_`, (4) output-<index>.
    assert_eq!(output_key(Some("DP-1"), Some("xdg"), "A", "B", 0), "DP-1");
    assert_eq!(output_key(None, Some("HDMI-A-1"), "A", "B", 0), "HDMI-A-1");
    assert_eq!(
        output_key(None, None, "Dell Inc.", "U2723QE", 0),
        "Dell_Inc._U2723QE"
    );
    assert_eq!(output_key(None, None, "", "", 3), "output-3");
    assert_eq!(output_key(Some("  "), None, "", "  ", 1), "output-1");
    // Never a leading dot, never a path separator, at most 64 bytes.
    assert_eq!(output_key(Some(".hidden"), None, "", "", 0), "_hidden");
    assert_eq!(output_key(Some("a/b\\c d"), None, "", "", 0), "a_b_c_d");
    assert_eq!(
        output_key(Some(&"x".repeat(100)), None, "", "", 0).len(),
        64
    );
    assert_eq!(
        output_key(Some("Bildschirm ä"), None, "", "", 0),
        "Bildschirm__"
    );

    // Duplicates get -2, -3 in the order of the global names; the result is
    // sorted by key, and every key is a name the arrangement accepts.
    let outputs = build_outputs(vec![
        raw(30, Some("DP-1"), "", "", [3840, 0]),
        raw(10, Some("DP-1"), "", "", [0, 0]),
        raw(20, Some("DP-1"), "", "", [1920, 0]),
        raw(40, None, "", "", [0, 1080]),
    ]);
    let keys: Vec<(u32, &str)> = outputs
        .iter()
        .map(|output| (output.global_name, output.key.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            (10, "DP-1"),
            (20, "DP-1-2"),
            (30, "DP-1-3"),
            (40, "output-3")
        ]
    );
    let mut layout = DesktopLayout::default();
    for output in &outputs {
        assert!(layout.monitor_mut(&output.key).is_ok(), "{}", output.key);
    }
    // A long name keeps the suffix inside 64 bytes.
    let long = "n".repeat(64);
    let outputs = build_outputs(vec![
        raw(1, Some(&long), "", "", [0, 0]),
        raw(2, Some(&long), "", "", [1, 0]),
    ]);
    assert_eq!(outputs[0].key.len(), 64);
    assert_eq!(outputs[1].key.len(), 64);
    assert!(outputs[1].key.ends_with("-2"));

    // The primary output is the one at the top left by (y, x).
    let outputs = build_outputs(vec![
        raw(1, Some("B"), "", "", [1920, 0]),
        raw(2, Some("A"), "", "", [0, 500]),
        raw(3, Some("C"), "", "", [0, 0]),
    ]);
    let primary: Vec<&str> = outputs
        .iter()
        .filter(|output| output.primary)
        .map(|output| output.key.as_str())
        .collect();
    assert_eq!(primary, vec!["C"]);
    let monitor = Monitor::from(outputs[0].clone());
    assert_eq!(monitor.rect, Some([0, 500, 1920, 1580]));
}

#[test]
fn desktop_task_layer_keysym_table() {
    assert_eq!(keysym_to_egui(0x61), Some(Key::A));
    assert_eq!(keysym_to_egui(0x5a), Some(Key::Z));
    assert_eq!(keysym_to_egui(0x30), Some(Key::Num0));
    assert_eq!(keysym_to_egui(0xffb7), Some(Key::Num7));
    assert_eq!(keysym_to_egui(0xff51), Some(Key::ArrowLeft));
    assert_eq!(keysym_to_egui(0xff52), Some(Key::ArrowUp));
    assert_eq!(keysym_to_egui(0xff53), Some(Key::ArrowRight));
    assert_eq!(keysym_to_egui(0xff54), Some(Key::ArrowDown));
    for (sym, key) in [
        (0xff50, Key::Home),
        (0xff57, Key::End),
        (0xff55, Key::PageUp),
        (0xff56, Key::PageDown),
        (0xff63, Key::Insert),
        (0xffff, Key::Delete),
        (0xff08, Key::Backspace),
        (0xff0d, Key::Enter),
        (0xff8d, Key::Enter),
        (0xff09, Key::Tab),
        (0xfe20, Key::Tab),
        (0xff1b, Key::Escape),
        (0x20, Key::Space),
        (0xffbe, Key::F1),
        (0xffc9, Key::F12),
        (0x2d, Key::Minus),
        (0x2b, Key::Plus),
        (0x3d, Key::Equals),
        (0x2c, Key::Comma),
        (0x2e, Key::Period),
        (0x2f, Key::Slash),
        (0x3b, Key::Semicolon),
        (0x5c, Key::Backslash),
        (0x5b, Key::OpenBracket),
        (0x5d, Key::CloseBracket),
        (0x27, Key::Quote),
        (0x60, Key::Backtick),
    ] {
        assert_eq!(keysym_to_egui(sym), Some(key), "{sym:#x}");
    }
    // Everything else is text only.
    assert_eq!(keysym_to_egui(0xffe1), None, "Shift_L");
    assert_eq!(keysym_to_egui(0xffca), None, "F13");
    assert_eq!(keysym_to_egui(0x20ac), None, "euro sign");

    let plain = modifiers_of(false, false, false);
    let ctrl = modifiers_of(false, true, false);
    let shift = modifiers_of(false, false, true);
    let press = |sym: u32, text: Option<&'static str>, modifiers: Modifiers| KeyPress {
        sym,
        text,
        pressed: true,
        repeat: false,
        modifiers,
    };
    // A letter: the key, then its text.
    let events = key_events(press(0x61, Some("a"), plain), String::new);
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        Event::Key {
            key: Key::A,
            pressed: true,
            repeat: false,
            ..
        }
    ));
    assert_eq!(events[1], Event::Text("a".into()));
    // A release gives the key and no text; a repeat is marked.
    let release = KeyPress {
        pressed: false,
        ..press(0x61, Some("a"), plain)
    };
    let events = key_events(release, String::new);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::Key { pressed: false, .. }));
    let again = KeyPress {
        repeat: true,
        ..press(0x61, Some("a"), plain)
    };
    assert!(matches!(
        key_events(again, String::new)[0],
        Event::Key { repeat: true, .. }
    ));
    // Control characters are never text; Enter is a key only.
    let events = key_events(press(0xff0d, Some("\r"), plain), String::new);
    assert_eq!(events.len(), 1);
    assert!(!is_typed_text("\u{7f}") && !is_typed_text("\u{1b}") && !is_typed_text(""));
    assert!(!is_typed_text("\u{e000}"));
    assert!(is_typed_text("ä") && is_typed_text("€"));
    // A key without an egui key still types its text (euro sign).
    assert_eq!(
        key_events(press(0x20ac, Some("€"), plain), String::new),
        vec![Event::Text("€".into())]
    );
    // Ctrl or Alt: no text.
    let events = key_events(press(0x61, Some("a"), ctrl), String::new);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::Key { key: Key::A, .. }));
    let alt = modifiers_of(true, false, false);
    assert_eq!(
        key_events(press(0x61, Some("a"), alt), String::new).len(),
        1
    );
    // Clipboard keys are events of their own, not keys.
    assert_eq!(
        key_events(press(0x63, Some("c"), ctrl), String::new),
        vec![Event::Copy]
    );
    assert_eq!(
        key_events(press(0x78, Some("x"), ctrl), String::new),
        vec![Event::Cut]
    );
    assert_eq!(
        key_events(press(0x76, Some("v"), ctrl), || "clip".to_string()),
        vec![Event::Paste("clip".into())]
    );
    assert_eq!(
        key_events(press(0x43, None, ctrl), String::new),
        vec![Event::Copy],
        "capital C with Ctrl"
    );
    assert_eq!(
        key_events(press(0xff63, None, ctrl), String::new),
        vec![Event::Copy]
    );
    assert_eq!(
        key_events(press(0xff63, None, shift), || String::new()),
        vec![Event::Paste(String::new())]
    );
    assert_eq!(
        key_events(press(0xffff, None, shift), String::new),
        vec![Event::Cut]
    );
    // The paste text is only read when it is needed; a repeat pastes nothing.
    let events = key_events(press(0x61, Some("a"), plain), || panic!("no paste here"));
    assert_eq!(events.len(), 2);
    let repeat = KeyPress {
        repeat: true,
        ..press(0x76, Some("v"), ctrl)
    };
    assert!(key_events(repeat, || panic!("no paste on repeat")).is_empty());
    // A released Ctrl+C is a plain key release.
    let release = KeyPress {
        pressed: false,
        ..press(0x63, None, ctrl)
    };
    assert!(matches!(
        key_events(release, String::new)[0],
        Event::Key {
            key: Key::C,
            pressed: false,
            ..
        }
    ));
    assert!(ctrl.command && !plain.command);
}

#[test]
fn layer_scale_and_pointer_mapping() {
    // Fractional scales are 120ths: round(logical × n / 120).
    let scale = OutputScale::Fraction(180);
    assert_eq!(scale.pixels_per_point(), 1.5);
    assert_eq!(scale.buffer_size((1280, 720)), (1920, 1080));
    assert_eq!(scale.buffer_size((1281, 721)), (1922, 1082));
    assert_eq!(OutputScale::Fraction(150).buffer_size((1000, 1)), (1250, 1));
    assert_eq!(scale.buffer_scale(), 1);
    assert_eq!(
        OutputScale::Integer(2).buffer_size((800, 600)),
        (1600, 1200)
    );
    assert_eq!(OutputScale::Integer(2).buffer_scale(), 2);
    assert_eq!(OutputScale::Integer(0).pixels_per_point(), 1.0);
    assert_eq!(OutputScale::Fraction(120).buffer_size((0, 5)), (0, 5));

    assert_eq!(button_of(0x110), Some(PointerButton::Primary));
    assert_eq!(button_of(0x111), Some(PointerButton::Secondary));
    assert_eq!(button_of(0x112), Some(PointerButton::Middle));
    assert_eq!(button_of(0x113), Some(PointerButton::Extra1));
    assert_eq!(button_of(0x114), Some(PointerButton::Extra2));
    assert_eq!(button_of(0x115), None);

    // A notch is one line, a smooth scroll is in points; both are negated.
    assert_eq!(
        wheel_delta((0.0, 0), (15.0, 1)),
        Some((MouseWheelUnit::Line, egui::vec2(0.0, -1.0)))
    );
    assert_eq!(
        wheel_delta((0.0, 0), (-12.5, 0)),
        Some((MouseWheelUnit::Point, egui::vec2(0.0, 12.5)))
    );
    assert_eq!(wheel_delta((0.0, 0), (0.0, 0)), None);

    assert_eq!(
        cursor_for(egui::CursorIcon::Default),
        Some(CursorIcon::Default)
    );
    assert_eq!(
        cursor_for(egui::CursorIcon::PointingHand),
        Some(CursorIcon::Pointer)
    );
    assert_eq!(cursor_for(egui::CursorIcon::Text), Some(CursorIcon::Text));
    assert_eq!(cursor_for(egui::CursorIcon::Grab), Some(CursorIcon::Grab));
    assert_eq!(
        cursor_for(egui::CursorIcon::Grabbing),
        Some(CursorIcon::Grabbing)
    );
    assert_eq!(
        cursor_for(egui::CursorIcon::ResizeHorizontal),
        Some(CursorIcon::EwResize)
    );
    assert_eq!(
        cursor_for(egui::CursorIcon::ResizeVertical),
        Some(CursorIcon::NsResize)
    );
    assert_eq!(
        cursor_for(egui::CursorIcon::NotAllowed),
        Some(CursorIcon::NotAllowed)
    );
    assert_eq!(
        cursor_for(egui::CursorIcon::Crosshair),
        Some(CursorIcon::Default)
    );
    assert_eq!(cursor_for(egui::CursorIcon::None), None);
}

fn monitors() -> Vec<Monitor> {
    vec![
        Monitor {
            key: "DP-1".into(),
            primary: true,
            rect: Some([0, 0, 1920, 1080]),
        },
        Monitor {
            key: "HDMI-A-1".into(),
            primary: false,
            rect: Some([1920, 0, 3200, 720]),
        },
    ]
}

fn host_on(key: &str, origin: [i32; 2]) -> LayerHost {
    LayerHost::new(key, monitors(), origin, egui::Context::default(), None)
}

#[test]
fn desktop_task_layer_cross_output_drop_uses_global_point() {
    let first = host_on("DP-1", [0, 0]);
    let second = host_on("HDMI-A-1", [1920, 0]);
    // The point is the output's origin plus the position in the surface,
    // rounded, in the logical coordinates of `Monitor::rect`.
    assert_eq!(first.global_point([100.4, 200.6]), Some([100, 201]));
    assert_eq!(second.global_point([10.0, 20.0]), Some([1930, 20]));
    // A pointer that left the first surface to the right lands on the second
    // output; one that left to the left of the second lands on the first.
    let point = first.global_point([1950.2, 500.0]).unwrap();
    assert_eq!(
        host_ui::monitor_at(&first, point).as_deref(),
        Some("HDMI-A-1")
    );
    let point = second.global_point([-30.0, 40.0]).unwrap();
    assert_eq!(host_ui::monitor_at(&second, point).as_deref(), Some("DP-1"));
    // Outside every output: none (the tile stays).
    let point = first.global_point([-40.0, 10.0]).unwrap();
    assert_eq!(host_ui::monitor_at(&first, point), None);
    let point = second.global_point([10.0, 900.0]).unwrap();
    assert_eq!(host_ui::monitor_at(&second, point), None);
    // The host names no work area (the surface is the work area), is a
    // Wayland window and names outputs by their key.
    assert_eq!(first.work_area(), None);
    assert_eq!(first.native_window(), Some(NativeWindow::Wayland));
    assert_eq!(first.monitor_name("DP-1").as_deref(), Some("DP-1"));
    assert_eq!(LayerHost::point_of([i32::MAX, 0], [10.0, 0.0])[0], i32::MAX);
}

#[test]
fn desktop_task_layer_namespace_by_desktop() {
    let facts = |desktop: &[&str], version: u32| SessionFacts {
        display: DisplayServer::Wayland,
        current_desktop: desktop.iter().map(|name| name.to_string()).collect(),
        layer_shell_version: Some(version),
        ..SessionFacts::default()
    };
    let decide = |facts: &SessionFacts| mechanism::decide(facts, ForeignDesktopIcons::Keep);
    // KDE: namespace `desktop`, so "Desktop anzeigen" keeps the surface.
    assert_eq!(
        decide(&facts(&["KDE"], 4)),
        Mechanism::LayerShell {
            keyboard: LayerKeyboard::OnDemand,
            namespace: LayerNamespace::Desktop
        }
    );
    assert_eq!(
        decide(&facts(&["ubuntu", "kde"], 3)),
        Mechanism::LayerShell {
            keyboard: LayerKeyboard::Off,
            namespace: LayerNamespace::Desktop
        }
    );
    for desktop in [&["sway"][..], &["Hyprland"], &["COSMIC"], &["niri"], &[]] {
        assert_eq!(
            decide(&facts(desktop, 4)),
            Mechanism::LayerShell {
                keyboard: LayerKeyboard::OnDemand,
                namespace: LayerNamespace::SmartExplorer
            },
            "{desktop:?}"
        );
    }
    assert_eq!(LayerNamespace::Desktop.as_str(), "desktop");
    assert_eq!(
        LayerNamespace::SmartExplorer.as_str(),
        "smart-explorer-desktop"
    );
    // Without the layer protocol the core never decides for this backend.
    let mut without = facts(&["sway"], 4);
    without.layer_shell_version = None;
    assert!(matches!(decide(&without), Mechanism::Unavailable(_)));
}
