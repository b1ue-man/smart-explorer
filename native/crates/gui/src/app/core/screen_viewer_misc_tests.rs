//! More acceptance of the desktop viewer (SCR-M7, U25-14): the target of a
//! start, the refusal of a changed device, the settings, the wgpu settings,
//! and the toolbar and the launch buttons as they are drawn.
use eframe::egui;

use super::screen_devices::{DeviceEntry, Devices};
use super::screen_prefs::{Fit, ViewerPrefs};
use super::screen_session::Shared;
use super::screen_target::{Relation, ScreenTarget, DEVICE_CHANGED};
use super::screen_toolbar_ui::{self, ToolbarInput};
use super::screen_viewer_app::wgpu_configuration;
use super::screen_viewer_fake::{hello, profiles, target, wait_for, Fake, DISPLAY};
use super::screen_viewer_view::{Common, ViewState};
use crate::screen::core::wire_hello::SessionMode;
use crate::share::access_grant::CapabilityDecision;
use crate::share::{PeerOpenTarget, ShareProfiles, ShareStatus};

#[test]
fn screen_task_viewer_target_parse() {
    let direct = ScreenTarget::parse(
        r#"{"relation":"direct","relation_id":"c1","device_id":"d1","source":null,"control":false}"#,
    )
    .unwrap();
    assert_eq!(direct.relation, Relation::Direct);
    assert_eq!(
        direct.to_peer().unwrap(),
        PeerOpenTarget::Direct {
            contact_id: "c1".into()
        }
    );
    assert!(direct.source.is_none() && !direct.control && !direct.fullscreen);

    let room = ScreenTarget::parse(
        r#"{"relation":"room","relation_id":"r1","device_id":"d2","source":"window:42","control":true,"fullscreen":true}"#,
    )
    .unwrap();
    assert_eq!(
        room.to_peer().unwrap(),
        PeerOpenTarget::RoomDevice {
            room_id: "r1".into(),
            device_id: "d2".into()
        }
    );
    assert_eq!(
        room.source.as_ref().map(ToString::to_string).as_deref(),
        Some("window:42")
    );
    assert!(room.control && room.fullscreen);

    // What se Desktop writes (U30) is what the viewer reads.
    let desktop = crate::desktop::core::screen_target::ScreenTarget::new(
        &PeerOpenTarget::RoomDevice {
            room_id: "r1".into(),
            device_id: "d2".into(),
        },
        "d2",
        true,
    );
    let parsed = ScreenTarget::parse(&desktop.to_arg().unwrap()).unwrap();
    assert_eq!(parsed.relation, Relation::Room);
    assert_eq!(parsed.device_id.as_deref(), Some("d2"));
    assert!(parsed.control && parsed.source.is_none());

    // The refusals.
    let room_without_device = r#"{"relation":"room","relation_id":"r1"}"#;
    assert_eq!(
        ScreenTarget::parse(room_without_device).unwrap_err(),
        "Das Raumgerät fehlt im Bildschirmziel"
    );
    for bad in [
        "",
        "[]",
        r#"{"relation":"cloud","relation_id":"x"}"#,
        r#"{"relation":"direct","relation_id":"  "}"#,
        r#"{"relation":"direct","relation_id":"c1","source":"desktop"}"#,
        r#"{"relation_id":"c1"}"#,
    ] {
        let error = ScreenTarget::parse(bad).unwrap_err();
        assert!(
            error.starts_with("Ungültiges Bildschirmziel"),
            "{bad:?}: {error}"
        );
    }
}

#[test]
fn screen_task_viewer_device_changed_refuses() {
    let known = profiles(ShareStatus::Available, Some("device-c1"));
    assert!(target().check_profiles(&known).is_ok());
    let swapped = profiles(ShareStatus::Available, Some("device-other"));
    assert_eq!(
        target().check_profiles(&swapped).unwrap_err(),
        DEVICE_CHANGED
    );
    let forgotten = profiles(ShareStatus::Available, None);
    assert_eq!(
        target().check_profiles(&forgotten).unwrap_err(),
        DEVICE_CHANGED
    );
    assert!(target().check_profiles(&ShareProfiles::default()).is_err());

    // The window refuses to open anything for a changed device.
    let fake = Fake::new(CapabilityDecision::Allow);
    let mut common = Common {
        backend: fake.backend(),
        devices: Devices::fixed(swapped),
        prefs: ViewerPrefs::default(),
        gpu: false,
        stats_on: false,
        low_latency_changed: false,
    };
    let mut state = ViewState::new(target(), false);
    state.ensure_session(&common, DISPLAY);
    assert!(state.session.is_none());
    assert_eq!(state.refused.as_deref(), Some(DEVICE_CHANGED));
    assert_eq!(state.status_text(), DEVICE_CHANGED);
    assert!(state.is_over());
    assert_eq!(fake.opens(), 0);

    // The device the target names opens exactly one session.
    common.devices = Devices::fixed(known);
    let mut state = ViewState::new(target(), false);
    state.ensure_session(&common, DISPLAY);
    state.ensure_session(&common, DISPLAY);
    wait_for("the session", || fake.opens() == 1);
    assert!(state.refused.is_none());
    let request = fake.inner.requests.lock().unwrap()[0].clone();
    assert_eq!(request.mode, SessionMode::View);
    assert_eq!(request.display_size, Some(DISPLAY));
}

#[test]
fn screen_task_viewer_prefs_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen").join("viewer.json");
    // Nothing stored, or unreadable: the defaults, without a message.
    assert_eq!(ViewerPrefs::load_from(&path), ViewerPrefs::default());
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, b"{ not json").unwrap();
    assert_eq!(ViewerPrefs::load_from(&broken), ViewerPrefs::default());

    let prefs = ViewerPrefs {
        low_latency: true,
        sharpness: true,
        lossless: true,
        fit: Fit::Actual,
        ..ViewerPrefs::default()
    };
    prefs.save_to(&path).unwrap();
    assert_eq!(ViewerPrefs::load_from(&path), prefs);
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        stored,
        serde_json::json!({
            "schema": 1,
            "low_latency": true,
            "sharpness": true,
            "lossless": true,
            "fit": "actual"
        })
    );
    // Saving again replaces the file; an older file with fewer settings
    // keeps the defaults for the rest.
    ViewerPrefs::default().save_to(&path).unwrap();
    assert_eq!(ViewerPrefs::load_from(&path), ViewerPrefs::default());
    let older = dir.path().join("older.json");
    std::fs::write(&older, br#"{"schema":1,"lossless":true}"#).unwrap();
    let older = ViewerPrefs::load_from(&older);
    assert!(older.lossless && !older.low_latency && older.fit == Fit::Fit);
}

#[test]
fn screen_task_viewer_wgpu_config() {
    use eframe::egui_wgpu::wgpu::PresentMode;
    let config = wgpu_configuration(false);
    assert_eq!(config.present_mode, PresentMode::AutoVsync);
    assert_eq!(config.desired_maximum_frame_latency, Some(1));
    let fast = wgpu_configuration(true);
    assert_eq!(fast.present_mode, PresentMode::AutoNoVsync);
    assert_eq!(fast.desired_maximum_frame_latency, Some(1));
    // `Mailbox` stays out: a surface without it ends the program.
    assert_ne!(config.present_mode, PresentMode::Mailbox);
    assert_ne!(fast.present_mode, PresentMode::Mailbox);
}

/// All text of the shapes a frame drew.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

fn draw(add: impl FnOnce(&mut egui::Ui)) -> Vec<String> {
    let mut add = Some(add);
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 300.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        if let Some(add) = add.take() {
            egui::CentralPanel::default().show(ctx, add);
        }
    });
    texts(&output.shapes)
}

#[test]
fn screen_task_viewer_launch_buttons() {
    use super::screen_launch::{all_devices_button, device_buttons, room_button, show_button};
    // „Bildschirm öffnen“ and „Bildschirm steuern“ need the device's ID and
    // its acceptance; whether it shares is the device's answer in the viewer.
    let [view, control] = device_buttons(Some("device-c1"), true);
    assert_eq!(
        (view.label, view.enabled, view.control),
        ("Bildschirm öffnen", true, false)
    );
    assert_eq!(
        (control.label, control.enabled, control.control),
        ("Bildschirm steuern", true, true)
    );
    let [no_id, _] = device_buttons(None, true);
    assert!(!no_id.enabled && !no_id.hint.is_empty());
    let [pending, _] = device_buttons(Some("device-c1"), false);
    assert!(!pending.enabled && !pending.hint.is_empty());
    assert!(!room_button(false).enabled);
    assert_eq!(room_button(true).label, "Bildschirme des Raums");
    assert_eq!(all_devices_button().label, "Alle Geräte");

    // They are drawn on the pages.
    let shown = draw(|ui| {
        for button in device_buttons(Some("device-c1"), true) {
            show_button(ui, &button);
        }
        show_button(ui, &room_button(true));
        show_button(ui, &all_devices_button());
    });
    for label in [
        "Bildschirm öffnen",
        "Bildschirm steuern",
        "Bildschirme des Raums",
        "Alle Geräte",
    ] {
        assert!(shown.iter().any(|text| text == label), "{label}: {shown:?}");
    }
}

#[test]
fn screen_task_viewer_toolbar_labels() {
    let shared = Shared::new(false, 0);
    shared.update(|info| info.hello = Some(hello(CapabilityDecision::Allow)));
    let entries = vec![DeviceEntry {
        target: target(),
        name: "Laptop".into(),
        room: None,
        online: true,
    }];
    let prefs = ViewerPrefs::default();
    let show = |controlled_by: Option<&str>| {
        draw(|ui| {
            let info = shared.info();
            let input = ToolbarInput {
                devices: &entries,
                current: &entries[0].target,
                current_name: "Laptop",
                info: Some(&*info),
                source: None,
                prefs: &prefs,
                control_on: controlled_by.is_none(),
                controlled_by,
                fullscreen: false,
                stats_on: false,
                low_latency_changed: false,
            };
            let _ = screen_toolbar_ui::show(ui, &input);
        })
    };
    let shown = show(None);
    for label in [
        "Gerät:",
        "Alle Geräte",
        "Quelle:",
        "Steuern",
        "Tasten ▾",
        "Einpassen",
        "1:1",
        "Vollbild",
        "Schärfe vor Flüssigkeit",
        "Verlustfrei",
        "Niedrigste Latenz (Bildrisse möglich)",
        "Statistik",
    ] {
        assert!(shown.iter().any(|text| text == label), "{label}: {shown:?}");
    }
    assert!(!shown.iter().any(|text| text.starts_with("Gesteuert von")));
    // Another device holds the host: the switch is off and says who.
    let taken = show(Some("Büro"));
    assert!(
        taken.iter().any(|text| text == "Gesteuert von Büro"),
        "{taken:?}"
    );
}
