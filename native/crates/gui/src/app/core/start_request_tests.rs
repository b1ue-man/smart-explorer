//! V9 start parser: every earlier invocation form keeps its route (F4), and
//! `--open-location`/`--open-page` have exact arguments whose values never
//! become a start path.
use super::*;

const WINDOWS: StartFacts = StartFacts {
    unregister_supported: true,
};
const LINUX: StartFacts = StartFacts {
    unregister_supported: false,
};

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(|item| OsString::from(*item)).collect()
}

fn gui(path: Option<&str>, updated: bool) -> StartRoute {
    StartRoute::Window {
        updated,
        request: StartRequest::Gui {
            start_path: path.map(PathBuf::from),
        },
    }
}

fn window(request: StartRequest) -> StartRoute {
    StartRoute::Window {
        updated: false,
        request,
    }
}

fn daemon(drive_restore: bool) -> StartRoute {
    StartRoute::SyncDaemon { drive_restore }
}

/// One example of every start kind, with values as se passes them.
fn every_request() -> Vec<StartRequest> {
    vec![
        StartRequest::AccessConsent {
            id: "f3a9c2d4e5b6a7c8".into(),
        },
        StartRequest::ScreenIndicator,
        StartRequest::ScreenHostWorker,
        StartRequest::ScreenView {
            target_json: r#"{"relation":"direct","relation_id":"c1","device_id":"d1","source":null,"control":false}"#.into(),
        },
        StartRequest::ScreenGrid { room: None },
        StartRequest::ScreenGrid {
            room: Some("raum-1".into()),
        },
        StartRequest::DesktopShell,
        StartRequest::DesktopSurface {
            target: SurfaceTarget::Monitor {
                key: "0123456789abcdef".into(),
            },
        },
        StartRequest::DesktopSurface {
            target: SurfaceTarget::Monitor {
                key: "HDMI-1".into(),
            },
        },
        StartRequest::DesktopSurface {
            target: SurfaceTarget::GnomeMonitor { index: 1 },
        },
        StartRequest::DesktopSurface {
            target: SurfaceTarget::Layer,
        },
        StartRequest::DesktopJumplistClear,
        StartRequest::OpenLocation {
            locator: "share://direct/c1/".into(),
        },
        StartRequest::OpenPage {
            page: StartPage::SettingsIntegration,
        },
        StartRequest::Gui { start_path: None },
        StartRequest::Gui {
            start_path: Some(PathBuf::from("/srv/daten")),
        },
    ]
}

#[test]
fn desktop_task_gui_start_order_unchanged() {
    // Every form that reached the decisions of `run_gui` up to v0.5.182
    // (`N/lib.rs:80-112`: installer, autostart, updater, context menus,
    // explorer command) with the route it took then.
    let table: &[(&[&str], StartFacts, StartRoute)] = &[
        (&[], WINDOWS, gui(None, false)),
        (&[], LINUX, gui(None, false)),
        (
            &[r"C:\Users\nutzer\Desktop"],
            WINDOWS,
            gui(Some(r"C:\Users\nutzer\Desktop"), false),
        ),
        (&[r"D:\"], WINDOWS, gui(Some(r"D:\"), false)),
        (
            &["/home/nutzer/Projekte"],
            LINUX,
            gui(Some("/home/nutzer/Projekte"), false),
        ),
        (&["/srv/a", "/srv/b"], LINUX, gui(Some("/srv/a"), false)),
        (&["-", "/srv/b"], LINUX, gui(Some("-"), false)),
        (&["--updated"], WINDOWS, gui(None, true)),
        (&["--updated"], LINUX, gui(None, true)),
        (
            &["--updated", "/srv/daten"],
            LINUX,
            gui(Some("/srv/daten"), true),
        ),
        (
            &["/srv/daten", "--updated"],
            LINUX,
            gui(Some("/srv/daten"), true),
        ),
        (&["--update-rollback"], WINDOWS, gui(None, false)),
        (
            &["--unbekannt", "/srv/daten"],
            LINUX,
            gui(Some("/srv/daten"), false),
        ),
        (&["--sync-guardian"], WINDOWS, StartRoute::SyncGuardian),
        (&["--sync-guardian"], LINUX, StartRoute::SyncGuardian),
        (
            &["--sync-guardian", "/srv/daten"],
            LINUX,
            gui(Some("/srv/daten"), false),
        ),
        (&["--sync-daemon"], WINDOWS, daemon(false)),
        (&["--sync-daemon"], LINUX, daemon(false)),
        (&["/srv/daten", "--sync-daemon"], LINUX, daemon(false)),
        (&["--sync-guardian", "--sync-daemon"], LINUX, daemon(false)),
        (&["--updated", "--sync-daemon"], WINDOWS, daemon(false)),
        (&["--unregister", "--sync-daemon"], WINDOWS, daemon(false)),
        (
            &["--unregister"],
            WINDOWS,
            StartRoute::Unregister { updated: false },
        ),
        (
            &["--updated", "--unregister"],
            WINDOWS,
            StartRoute::Unregister { updated: true },
        ),
        (
            &["/srv/daten", "--unregister"],
            WINDOWS,
            StartRoute::Unregister { updated: false },
        ),
        (&["--unregister"], LINUX, gui(None, false)),
        (
            &["--unregister", "/srv/daten"],
            LINUX,
            gui(Some("/srv/daten"), false),
        ),
        // S2 start reason: only with `--sync-daemon`, never a start path.
        (&["--sync-daemon", "--drive-restore"], LINUX, daemon(true)),
        (&["--drive-restore", "--sync-daemon"], WINDOWS, daemon(true)),
        (&["--drive-restore"], LINUX, gui(None, false)),
    ];
    for (form, facts, expected) in table {
        let found = route(&args(form), *facts);
        assert_eq!(found, Ok(expected.clone()), "{form:?} {facts:?}");
    }

    // The hooks before `route` keep their own detection (`main.rs`,
    // `local_access`, `share`): none of their switches is a V9 kind.
    for switch in [
        "--mount-host",
        "--local-read-helper",
        "--storage-analysis-admin",
        "--share-exec-supervisor",
        "--sync-guardian",
        "--sync-daemon",
        "--drive-restore",
        "--updated",
        "--update-rollback",
        "--unregister",
    ] {
        assert!(!is_start_switch(OsStr::new(switch)), "{switch}");
    }

    // New kinds sit after `--sync-daemon`: background kinds before the update
    // confirmation, window kinds after the updater steps.
    for request in every_request() {
        let form = request.to_args();
        let background = request.is_background();
        let expected = if background {
            StartRoute::Background(request.clone())
        } else {
            window(request.clone())
        };
        assert_eq!(route(&form, WINDOWS), Ok(expected.clone()), "{form:?}");
        assert_eq!(route(&form, LINUX), Ok(expected), "{form:?}");
        let mut with_daemon = form.clone();
        with_daemon.push("--sync-daemon".into());
        assert_eq!(route(&with_daemon, LINUX), Ok(daemon(false)), "{form:?}");
    }
    let background: Vec<bool> = every_request()
        .iter()
        .map(StartRequest::is_background)
        .collect();
    assert_eq!(
        background,
        [
            false, true, true, false, false, false, true, true, true, true, true, true, false,
            false, false, false
        ]
    );

    // A V9 switch counts only in first position; elsewhere the old rule holds.
    assert_eq!(
        route(&args(&["/srv/daten", "--desktop-shell"]), LINUX),
        Ok(gui(Some("/srv/daten"), false))
    );
    assert_eq!(
        route(&args(&["--updated", "--screen-host-worker"]), LINUX),
        Ok(gui(None, true))
    );
    // `--screen-host-worker` only as the only argument.
    assert!(route(&args(&["--screen-host-worker", "/srv/daten"]), LINUX).is_err());
}

#[test]
fn desktop_task_open_location_args() {
    // Exact arguments, read back to the same request.
    for (locator, form) in [
        (
            "share://direct/c1/",
            ["--open-location", "share://direct/c1/"],
        ),
        (
            "share://room/r1/d1/",
            ["--open-location", "share://room/r1/d1/"],
        ),
        (
            "vault://0123abcd/",
            ["--open-location", "vault://0123abcd/"],
        ),
    ] {
        let request = StartRequest::OpenLocation {
            locator: locator.into(),
        };
        assert_eq!(request.to_args(), args(&form));
        assert_eq!(parse(&args(&form)), Ok(request));
    }
    for (page, text) in [
        (StartPage::ShareDevices, "share-devices"),
        (StartPage::ShareRooms, "share-rooms"),
        (
            StartPage::ShareDevice {
                contact_id: "c1".into(),
            },
            "share-device:c1",
        ),
        (
            StartPage::ShareRoom {
                room_id: "r1".into(),
            },
            "share-room:r1",
        ),
        (StartPage::SettingsIntegration, "settings-integration"),
    ] {
        let request = StartRequest::OpenPage { page };
        assert_eq!(request.to_args(), args(&["--open-page", text]));
        assert_eq!(parse(&request.to_args()), Ok(request));
    }
    for request in every_request() {
        assert_eq!(
            parse(&request.to_args()),
            Ok(request.clone()),
            "{request:?}"
        );
    }

    // A value is never the local start path, wherever it stands.
    assert_eq!(
        route(&args(&["--open-location", "/home/nutzer"]), LINUX),
        Ok(window(StartRequest::OpenLocation {
            locator: "/home/nutzer".into()
        }))
    );
    assert_eq!(
        route(
            &args(&["--updated", "--open-location", "/home/nutzer"]),
            LINUX
        ),
        Ok(gui(None, true))
    );
    assert_eq!(
        route(
            &args(&["--updated", "--open-page", "share-rooms", "/srv/daten"]),
            WINDOWS
        ),
        Ok(gui(Some("/srv/daten"), true))
    );
    // `--sync-daemon` keeps its place and meaning.
    assert_eq!(
        route(
            &args(&["--open-location", "share://direct/c1/", "--sync-daemon"]),
            LINUX
        ),
        Ok(daemon(false))
    );

    // Malformed forms are errors, never a window at a surprising place.
    let missing = |switch| StartArgError {
        switch,
        problem: ArgProblem::MissingValue,
    };
    assert_eq!(
        parse(&args(&["--open-location"])),
        Err(missing(OPEN_LOCATION))
    );
    assert_eq!(
        parse(&args(&["--open-location", "--updated"])),
        Err(missing(OPEN_LOCATION))
    );
    assert_eq!(parse(&args(&["--open-page"])), Err(missing(OPEN_PAGE)));
    let invalid: &[&[&str]] = &[
        &["--open-location", ""],
        &["--open-page", "share-device:"],
        &["--open-page", "share-room:a b"],
        &["--open-page", "einstellungen"],
        &["--desktop-surface", "--monitor", "../x"],
        &["--desktop-surface", "--gnome-monitor", "eins"],
        &["--desktop-surface"],
        &["--screen-view", "kein json"],
        &["--screen-view", "[1,2]"],
    ];
    for form in invalid {
        assert!(
            matches!(
                parse(&args(form)),
                Err(StartArgError {
                    problem: ArgProblem::Invalid(_),
                    ..
                })
            ),
            "{form:?}"
        );
    }
    let unexpected: &[&[&str]] = &[
        &["--open-location", "share://direct/c1/", "/srv/daten"],
        &["--open-page", "share-devices", "--updated"],
        &["--screen-grid", "--monitor", "x"],
        &["--desktop-shell", "--layer"],
    ];
    for form in unexpected {
        assert!(
            matches!(
                parse(&args(form)),
                Err(StartArgError {
                    problem: ArgProblem::Unexpected(_),
                    ..
                })
            ),
            "{form:?}"
        );
    }
}
