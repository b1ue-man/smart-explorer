//! Acceptance of the Linux screen host that needs no display (SCR-M5,
//! stage `linux-core: unit`): the session table, the portal's single
//! monitor, the limits, a system without PipeWire, the app-unit names and
//! the token file.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::session_state::SessionState;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn screen_task_session_detect_table() {
    let table: [(&[(&str, &str)], SessionKind); 9] = [
        (&[], SessionKind::None),
        (&[("DISPLAY", ":0")], SessionKind::X11),
        (&[("DISPLAY", "")], SessionKind::None),
        (&[("WAYLAND_DISPLAY", "wayland-0")], SessionKind::Wayland),
        (&[("WAYLAND_DISPLAY", "")], SessionKind::None),
        (&[("XDG_SESSION_TYPE", "wayland")], SessionKind::Wayland),
        (&[("XDG_SESSION_TYPE", "x11")], SessionKind::None),
        // XWayland: both set, the portal decides.
        (
            &[("DISPLAY", ":1"), ("WAYLAND_DISPLAY", "wayland-0")],
            SessionKind::Wayland,
        ),
        (
            &[("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "wayland")],
            SessionKind::Wayland,
        ),
    ];
    for (pairs, expected) in table {
        assert_eq!(
            session_detect::detect(&env_of(pairs)),
            expected,
            "{pairs:?}"
        );
    }
    // A process without display (a root worker): no screen at all.
    let backend = LinuxBackend::with(SessionKind::None, pipewire_abi::LIBRARY_NAME, None);
    assert!(!backend.graphical_session());
    assert!(backend.monitors().is_empty());
    let refusal = backend
        .open_source(&SourceId::Monitor("DP-1".into()), None)
        .err()
        .expect("no display");
    assert_eq!(refusal.reason, "no_display");
    assert!(session_detect::is_wlroots_desktop("sway"));
    assert!(session_detect::is_wlroots_desktop("Hyprland:wlroots"));
    assert!(!session_detect::is_wlroots_desktop("GNOME"));
}

#[test]
fn screen_task_wayland_monitors_single_portal_entry() {
    let backend = LinuxBackend::with(SessionKind::Wayland, pipewire_abi::LIBRARY_NAME, None);
    assert_eq!(
        backend.monitors(),
        vec![MonitorInfo {
            id: "portal".into(),
            name: "Auswahl am Gerät".into(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            primary: true,
        }]
    );
    for other in [
        SourceId::Monitor("DP-1".into()),
        SourceId::Window("42".into()),
    ] {
        let refusal = backend.open_source(&other, None).err().expect("refused");
        assert_eq!(refusal.reason, PORTAL_SELECTS_SOURCE);
    }
    // The test picture stays what it was.
    use crate::screen::core::host_backend::HostOutputPlan;
    use crate::screen::core::host_codec_adapters::{HostCodecPlan, HostContract};
    let admission = prepared::test_admission("linux-synthetic");
    let prepared = backend
        .prepare_session_source(
            "fixture",
            "linux-synthetic",
            &SourceId::Synthetic,
            Some(FrameSize::new(320, 180)),
            false,
            admission.clone(),
        )
        .ok()
        .expect("synthetic prepares");
    let output = HostOutputPlan::new(
        HostContract::Legacy,
        FrameSize::new(320, 180),
        HostCodecPlan::Tiles,
    )
    .unwrap();
    let grant = admission
        .authority()
        .approve(prepared.capture_plan(&output).unwrap().unwrap())
        .unwrap();
    let mut synthetic = prepared.activate(grant).ok().expect("synthetic activates");
    assert!(matches!(
        synthetic.capture(1, true).unwrap(),
        SourceFrame::Frame(_)
    ));
}

#[test]
fn screen_task_limits_consent_each_session_without_token() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(portal_tokens::FILE_NAME);
    let backend = LinuxBackend::with(
        SessionKind::Wayland,
        pipewire_abi::LIBRARY_NAME,
        Some(file.clone()),
    );
    assert!(backend
        .limits()
        .contains(&LIMIT_CONSENT_EACH_SESSION.to_string()));
    portal_tokens::save_to(&file, TokenKind::ScreenCast, "restore-1", 100).unwrap();
    let limits = backend.limits();
    assert!(!limits.contains(&LIMIT_CONSENT_EACH_SESSION.to_string()));
    assert!(limits.contains(&LIMIT_WINDOW_PICK_ON_HOST.to_string()));
    // Every limit fits into the 64 bytes of the wire.
    assert!(limits.iter().all(|limit| limit.len() <= 64));
    // A remote-desktop token alone is no screen grant.
    let only_input = directory.path().join("input.json");
    portal_tokens::save_to(&only_input, TokenKind::RemoteDesktop, "input-1", 100).unwrap();
    let other = LinuxBackend::with(
        SessionKind::Wayland,
        pipewire_abi::LIBRARY_NAME,
        Some(only_input),
    );
    assert!(other
        .limits()
        .contains(&LIMIT_CONSENT_EACH_SESSION.to_string()));
}

#[test]
fn screen_task_portal_tokens_roundtrip_and_forget() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(portal_tokens::FILE_NAME);
    assert_eq!(
        portal_tokens::load_from(&file),
        portal_tokens::Tokens::default()
    );
    portal_tokens::save_to(&file, TokenKind::ScreenCast, "one", 10).unwrap();
    portal_tokens::save_to(&file, TokenKind::RemoteDesktop, "two", 20).unwrap();
    // A new screen cast token replaces the old one and keeps the other kind.
    portal_tokens::save_to(&file, TokenKind::ScreenCast, "three", 30).unwrap();
    let tokens = portal_tokens::load_from(&file);
    let screen = tokens.get(TokenKind::ScreenCast).unwrap();
    assert_eq!((screen.token.as_str(), screen.saved_at), ("three", 30));
    let input = tokens.get(TokenKind::RemoteDesktop).unwrap();
    assert_eq!((input.token.as_str(), input.saved_at), ("two", 20));
    let text = std::fs::read_to_string(&file).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["screencast"]["token"], "three");
    assert_eq!(json["remote_desktop"]["saved_at"], 20);
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Both kinds go, in the order screencast, remote_desktop; the file too.
    let removed = portal_tokens::forget_at(&file).unwrap();
    assert_eq!(
        removed,
        vec![TokenKind::ScreenCast, TokenKind::RemoteDesktop]
    );
    assert!(!file.exists());
    assert!(portal_tokens::forget_at(&file).unwrap().is_empty());
    // A file that is no JSON counts as none and is replaced by the next save.
    std::fs::write(&file, b"not json").unwrap();
    assert_eq!(
        portal_tokens::load_from(&file),
        portal_tokens::Tokens::default()
    );
    portal_tokens::save_to(&file, TokenKind::ScreenCast, "fresh", 40).unwrap();
    assert_eq!(
        portal_tokens::load_from(&file)
            .get(TokenKind::ScreenCast)
            .unwrap()
            .token,
        "fresh"
    );
    // Only one kind stored: only that one is reported.
    portal_tokens::forget_at(&file).unwrap();
    portal_tokens::save_to(&file, TokenKind::RemoteDesktop, "only", 50).unwrap();
    assert_eq!(
        portal_tokens::forget_at(&file).unwrap(),
        vec![TokenKind::RemoteDesktop]
    );
}

#[test]
fn screen_task_linux_without_pipewire_starts() {
    let missing = "libgibtsnicht.so.0";
    assert!(pipewire_abi::load_from(missing).is_err());
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(portal_tokens::FILE_NAME);
    let backend = LinuxBackend::with(SessionKind::Wayland, missing, Some(file));
    assert!(backend
        .limits()
        .contains(&LIMIT_PIPEWIRE_MISSING.to_string()));
    // Preparation returns before the dialog; unavailable PipeWire is
    // reported without activating an unfinanced capture source.
    let admission = prepared::test_admission("linux-missing-pipewire");
    let source = backend
        .prepare_session_source(
            "fixture",
            "linux-missing-pipewire",
            &SourceId::Monitor(PORTAL_MONITOR_ID.into()),
            None,
            false,
            admission,
        )
        .ok()
        .expect("portal prepares");
    let output = crate::screen::core::host_backend::HostOutputPlan::new(
        crate::screen::core::host_codec_adapters::HostContract::Legacy,
        FrameSize::new(320, 180),
        crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles,
    )
    .unwrap();
    let started = Instant::now();
    loop {
        match source.capture_plan(&output) {
            Err(error) => {
                assert!(error.to_string().contains("pipewire_missing"));
                break;
            }
            Ok(None) => {}
            Ok(Some(_)) => panic!("missing PipeWire cannot offer native buffers"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "still waiting for missing PipeWire"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn screen_task_app_units_parse() {
    use app_units::{parse_app_unit, AppUnit};
    let cases = [
        (
            "app-gnome-org.gnome.Evince-12345.scope",
            Some(("gnome", "org.gnome.Evince")),
        ),
        (
            "app-KDE-org.kde.okular@12345.service",
            Some(("KDE", "org.kde.okular")),
        ),
        (
            "app-org.mozilla.firefox-1234.scope",
            Some(("", "org.mozilla.firefox")),
        ),
        ("app-firefox-1234.scope", Some(("", "firefox"))),
    ];
    for (name, expected) in cases {
        let expected = expected.map(|(launcher, app_id)| AppUnit {
            launcher: (!launcher.is_empty()).then(|| launcher.to_string()),
            app_id: app_id.to_string(),
        });
        assert_eq!(parse_app_unit(name), expected, "{name}");
    }
    for other in [
        "session-3.scope",
        "app.slice",
        "app-firefox-xyz.scope",
        "app-firefox-1234.mount",
        "app-.service",
    ] {
        assert_eq!(parse_app_unit(other), None, "{other}");
    }
    // systemd escapes `-` in an application id.
    assert_eq!(
        parse_app_unit("app-org.example.my\\x2dapp-77.scope").map(|unit| unit.app_id),
        Some("org.example.my-app".to_string())
    );
    // Active units only, each app once, named by the desktop file.
    let directory = tempfile::tempdir().unwrap();
    let applications = directory.path().join("applications");
    std::fs::create_dir_all(&applications).unwrap();
    std::fs::write(
        applications.join("org.gnome.Evince.desktop"),
        "[Desktop Action x]\nName=Wrong\n[Desktop Entry]\nName[de]=Dokumente\nName=Document Viewer\n",
    )
    .unwrap();
    let dirs = vec![directory.path().to_path_buf()];
    let units: Vec<(String, String)> = [
        ("app-gnome-org.gnome.Evince-1.scope", "active"),
        ("app-gnome-org.gnome.Evince-2.scope", "active"),
        ("app-firefox-3.scope", "active"),
        ("app-gimp-4.scope", "inactive"),
    ]
    .iter()
    .map(|(name, state)| (name.to_string(), state.to_string()))
    .collect();
    let apps = app_units::apps_of_units(&units, &|app_id| app_units::desktop_name(app_id, &dirs));
    let listed: Vec<(&str, &str)> = apps
        .iter()
        .map(|app| (app.app_id.as_str(), app.name.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("org.gnome.Evince", "Document Viewer"),
            ("firefox", "firefox")
        ]
    );
}

/// Sway (wlroots) has no RemoteDesktop portal: the host offers no input
/// and refuses it with `control_unavailable` (SCR-M6, NR N11); GNOME and KDE
/// offer pointer, keys and text.
#[test]
fn screen_task_wayland_control_unavailable_reported() {
    let none = input_for(SessionKind::Wayland, "sway");
    assert_eq!(none, InputCapabilities::default());
    assert!(!remote_desktop_portal("sway:wlroots"));
    let gnome = input_for(SessionKind::Wayland, "ubuntu:GNOME");
    assert!(gnome.pointer && gnome.keyboard && gnome.text);
    assert!(input_for(SessionKind::X11, "").keyboard);
    let backend = LinuxBackend::with(SessionKind::Wayland, "libpipewire-0.3-missing.so", None);
    let tap = [InputEvent::Text { text: "x".into() }];
    assert_eq!(backend.inject(&tap), Err(ControlBlock::ControlUnavailable));
    let headless = LinuxBackend::with(SessionKind::None, "libpipewire-0.3-missing.so", None);
    assert_eq!(headless.inject(&tap), Err(ControlBlock::UnsupportedOnHost));
}
