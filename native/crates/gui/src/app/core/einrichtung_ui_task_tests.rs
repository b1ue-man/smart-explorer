//! Task tests of WP3 (design 10.3, U1 and U4) for the desktop surfaces:
//! the device page offers Remote Desktop as one button per handling, the
//! feed projection every surface shares, the forwarding request without
//! acknowledgement or lifetime, and the programs page's clarification
//! without clicks. Prefix `einrichtung_task_`.
use super::*;
use crate::apps::{LaunchReceipt, LaunchState};
use crate::desktop::core::tiles::{
    project_feed, rdp_entries, service_url, FeedFacts, TileFeed, TileService, RDP_LABEL,
    RDP_NOT_STARTABLE, SERVICES_NONE,
};
use crate::rdp::core::{PublishedApp, RdpFeedSummary, SetupState};
use crate::share::loopback_services::{BindScope, LoopbackService, OwnerClass};
use std::net::{Ipv4Addr, SocketAddr};

fn app(alias: &str, name: &str) -> PublishedApp {
    PublishedApp {
        alias: alias.into(),
        display_name: name.into(),
        collection: "Apps".into(),
    }
}

fn summary(startable: bool, setup: SetupState, apps: Vec<PublishedApp>) -> RdpFeedSummary {
    RdpFeedSummary {
        startable,
        setup,
        remote_apps: apps,
    }
}

fn labels(entries: &[RdpEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.label.as_str()).collect()
}

fn text_shapes(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, result: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => result.push(text.galley.text().to_string()),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, result)),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in shapes {
        walk(&shape.shape, &mut result);
    }
    result
}

/// U1: a startable host renders exactly one „Remote Desktop“ button and one
/// button per published alias; nothing else is drawn (no checkbox, no field).
#[test]
fn einrichtung_task_ui_rdp_startable_renders_one_button_per_handling() {
    let rdp = summary(
        true,
        SetupState::Done,
        vec![app("calc", "Rechner"), app("word", "Word")],
    );
    let feed = TileFeed {
        known: true,
        rdp: Some(rdp),
        control: true,
        ..TileFeed::default()
    };
    let (entries, note) = page_entries(Some(&feed));
    assert_eq!(
        labels(&entries),
        [RDP_LABEL, "Rechner (RemoteApp)", "Word (RemoteApp)"]
    );
    assert_eq!(entries[0].remote_app, None);
    assert_eq!(entries[1].remote_app.as_deref(), Some("calc"));
    assert_eq!(note, None);
    let ctx = egui::Context::default();
    let mut clicked = None;
    let mut drawn = Vec::new();
    for _ in 0..2 {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                clicked = entry_buttons(ui, &entries, true);
            });
        });
        drawn = text_shapes(&output.shapes);
    }
    assert_eq!(drawn, labels(&entries));
    assert_eq!(clicked, None, "drawing never starts Remote Desktop");
}

/// The setup states of design 3.4: possible setups keep the one handling
/// with their text, an impossible one offers none, and no feed (older or
/// absent device) still offers the handling whose click reports the truth.
#[test]
fn einrichtung_task_ui_rdp_setup_states_are_shown_truthfully() {
    let (entries, note) = rdp_entries(None);
    assert_eq!(labels(&entries), [RDP_LABEL]);
    assert_eq!(note, None);
    let declined = summary(false, SetupState::Declined, vec![app("calc", "Rechner")]);
    let (entries, note) = rdp_entries(Some(&declined));
    assert_eq!(
        labels(&entries),
        [RDP_LABEL],
        "RemoteApps only when startable"
    );
    assert_eq!(note, Some(SetupState::Declined.text()));
    let failed = summary(false, SetupState::Failed("no_listener".into()), vec![]);
    assert_eq!(rdp_entries(Some(&failed)).1, Some(failed.setup.text()));
    let done = summary(false, SetupState::Done, vec![]);
    assert_eq!(
        rdp_entries(Some(&done)).1.as_deref(),
        Some(RDP_NOT_STARTABLE)
    );
    let home = summary(false, SetupState::Impossible("windows_home".into()), vec![]);
    let (entries, note) = rdp_entries(Some(&home));
    assert!(entries.is_empty());
    assert!(note.is_some_and(|text| text.contains("Windows Home")));
    let tile = TileFeed {
        rdp: None,
        ..TileFeed::default()
    };
    assert!(
        tile.tile_rdp_entries().is_empty(),
        "tile menus need a state"
    );
}

fn service(port: u16, scope: BindScope, process: Option<&str>) -> LoopbackService {
    LoopbackService {
        port,
        scope,
        owner: OwnerClass::NormalUser {
            uid_or_sid: "1000".into(),
        },
        process: process.map(str::to_string),
        pid: Some(4242),
    }
}

/// The feed projection of design 4.3/5.2: web ports become links at the
/// device address, a taken port shows its substitute, an unmapped port says
/// so, and steering is recognized from services or the RDP state.
#[test]
fn einrichtung_task_ui_feed_projection_lists_services_and_addresses() {
    let device = Ipv4Addr::new(127, 23, 45, 67);
    let services = vec![
        service(8080, BindScope::LoopbackOnly, Some("python3")),
        service(5432, BindScope::Wildcard, Some("postgres")),
        service(6000, BindScope::LoopbackOnly, None),
    ];
    let mapped = |port: u16| match port {
        8080 => Some(SocketAddr::from((device, 8080))),
        5432 => Some(SocketAddr::from((device, 15432))),
        _ => None,
    };
    let facts = FeedFacts {
        families: None,
        families_unavailable: Some("Wayland: nur Portalauswahl"),
        services: Some(services.as_slice()),
        services_unavailable: None,
        rdp: None,
        unavailable: None,
    };
    let feed = project_feed(facts, mapped);
    assert!(feed.known && feed.control);
    let lines: Vec<String> = feed.services.iter().map(TileService::line).collect();
    assert_eq!(lines[0], "python3 – http://127.23.45.67:8080");
    assert_eq!(
        lines[1],
        "postgres – 5432 → 127.23.45.67:15432 (Port belegt)"
    );
    assert_eq!(
        lines[2],
        "Port 6000 – Port 6000 (Adresse wird eingerichtet)"
    );
    assert_eq!(
        feed.services[0].target().as_deref(),
        Some("http://127.23.45.67:8080")
    );
    assert_eq!(
        feed.services[1].target().as_deref(),
        Some("127.23.45.67:15432")
    );
    assert_eq!(feed.notes, ["Wayland: nur Portalauswahl"]);
    assert_eq!(feed.summary().as_deref(), Some("3 Dienste"));
    let none = project_feed(
        FeedFacts {
            families: None,
            families_unavailable: None,
            services: Some(&[][..]),
            services_unavailable: None,
            rdp: None,
            unavailable: None,
        },
        |_| None,
    );
    assert_eq!(none.notes, [SERVICES_NONE]);
    let old = project_feed(
        FeedFacts {
            families: None,
            families_unavailable: None,
            services: None,
            services_unavailable: None,
            rdp: None,
            unavailable: Some("Gerät hat eine ältere Version"),
        },
        |_| None,
    );
    assert!(!old.known && !old.control);
    assert_eq!(
        service_url(SocketAddr::from((device, 443)), 443).as_deref(),
        Some("https://127.23.45.67:443")
    );
    assert_eq!(service_url(SocketAddr::from((device, 22)), 22), None);
    // The web ports of `share::auto_forward::service_url`: no link for a
    // database or VNC port of the 5000s.
    for port in [5432, 5672, 5900] {
        assert_eq!(service_url(SocketAddr::from((device, port)), port), None);
    }
    assert_eq!(
        service_url(SocketAddr::from((device, 5173)), 5173).as_deref(),
        Some("http://127.23.45.67:5173")
    );
}

/// Design 4.5: the further forwarding needs no acknowledgement field and no
/// lifetime field; it asks for no fixed lifetime.
#[test]
fn einrichtung_task_ui_manual_forward_has_no_acknowledgement_or_lifetime() {
    let request = crate::app::share_tcp_ui::manual_request("loopback", "5432", "0").unwrap();
    assert!(request.acknowledge_local_access);
    assert_eq!(request.lifetime_secs, 0);
    assert_eq!(request.fixed_lifetime(), None);
    let lan = crate::app::share_tcp_ui::manual_request("192.168.1.20", "80", "8080").unwrap();
    assert_eq!(lan.local_port, 8080);
    assert!(crate::app::share_tcp_ui::manual_request("loopback", "", "0").is_err());
    assert_eq!(
        crate::app::share_tcp_ui::admission_text(Some(
            crate::share::tcp_forward::ListenerAdmission::SameAccount
        )),
        "nur Programme deines Benutzerkontos"
    );
    assert_eq!(
        sessions::lifetime_text(None),
        "ohne feste Frist (endet mit dem RDP-Programm)"
    );
}

/// No local RDP program: the page says why instead of offering a dead
/// button.
#[test]
fn einrichtung_task_ui_missing_client_is_reported() {
    assert!(client_gap(&[]).is_some_and(|text| text.contains("freerdp3-sdl")));
}

fn receipt(state: LaunchState) -> LaunchReceipt {
    let request = crate::apps::LaunchRequest {
        request_id: "einrichtung-request".to_string().try_into().unwrap(),
        target_device_id: "remote-device".to_string().try_into().unwrap(),
        app_id: "app-id".to_string().try_into().unwrap(),
        user_id: "remote-user".to_string().try_into().unwrap(),
        session_id: "login-incarnation".to_string().try_into().unwrap(),
        catalog_revision: "catalog-revision".into(),
    };
    LaunchReceipt {
        request,
        launch_id: "launch".into(),
        dispatch_id: "dispatch".into(),
        state,
        reason: String::new(),
        pid: None,
        window: None,
    }
}

/// U4: an unclear outcome asks the same receipt without a click, at once
/// and again on each host event only; a final receipt ends the asking; the
/// catalog loads by itself once the device is present.
#[test]
fn einrichtung_task_ui_programs_clarify_without_click() {
    use crate::app::share_apps_ui::{auto_work, unresolved, AutoWork, HostEvent};
    let unknown = receipt(LaunchState::OutcomeUnknown);
    assert!(unresolved(true, None));
    assert!(unresolved(true, Some(&unknown)));
    assert!(unresolved(
        true,
        Some(&receipt(LaunchState::StartedWindowPending))
    ));
    for done in [
        LaunchState::Started,
        LaunchState::Activated,
        LaunchState::Denied,
        LaunchState::Unsupported,
    ] {
        assert!(!unresolved(true, Some(&receipt(done))));
    }
    assert!(!unresolved(false, None));
    let present = HostEvent {
        present: true,
        windows: Vec::new(),
    };
    let absent = HostEvent::default();
    assert_eq!(
        auto_work(&present, 1, false, None, true, None, false),
        AutoWork {
            catalog: true,
            receipt: true
        }
    );
    assert_eq!(
        auto_work(&present, 1, true, Some(1), true, Some(1), false),
        AutoWork::default(),
        "the same event never asks twice"
    );
    assert_eq!(
        auto_work(&present, 2, true, Some(1), true, Some(1), false),
        AutoWork {
            catalog: false,
            receipt: true
        }
    );
    assert_eq!(
        auto_work(&present, 2, true, Some(1), false, Some(1), true),
        AutoWork {
            catalog: true,
            receipt: false
        },
        "a start waiting for the unlocked session reads the catalog again"
    );
    assert_eq!(
        auto_work(&absent, 3, false, None, true, None, false),
        AutoWork::default()
    );
}

/// se Desktop starts Remote Desktop through `se rdp open` with the exact
/// relation endpoint and reports a failure as a notice.
#[test]
fn einrichtung_task_ui_surface_starts_rdp_through_se() {
    let args = crate::desktop::os::open_target::rdp_args("share://direct/c1/", Some("calc"));
    assert_eq!(
        args,
        [
            "rdp",
            "open",
            "share://direct/c1/",
            "--remote-app",
            "calc",
            "--notify-errors"
        ]
    );
    let plain = crate::desktop::os::open_target::rdp_args("share://direct/c1/", None);
    assert_eq!(
        plain,
        ["rdp", "open", "share://direct/c1/", "--notify-errors"]
    );
}
