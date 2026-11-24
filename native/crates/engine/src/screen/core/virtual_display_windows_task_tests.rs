//! Task tests of the Windows virtual display rules (plan 2026-11-17 P5,
//! `virtual_display_windows.rs`): the driver's mode list, the listed mode
//! for a wish, the desktop layout with the virtual monitor as primary, what
//! counts as a physical monitor, when the elevated step signs again, and
//! the install at use time (user rule 2026-11-24).
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::windows_driver::*;
use super::{
    chain_text, DriverLimit, VirtualDisplayChoice, VirtualDisplayReport, VirtualDisplayState,
    VirtualDisplayWish, SIZE_CHOICES,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::set_display::Shown;
use crate::screen::core::virtual_display_host::{
    self as host, capability, DisplayProbe, SessionStart, SetupProgress, VirtualDisplayDriver,
};
use crate::screen::core::wire_hello::SessionMode;

fn size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width, height)
}

fn listed(sizes: &[FrameSize]) -> Vec<ListedMode> {
    sizes
        .iter()
        .flat_map(|size| {
            REFRESH_RATES.iter().map(|rate| ListedMode {
                size: *size,
                refresh_hz: *rate,
            })
        })
        .collect()
}

/// The settings list every size of the viewer's menu at every rate, one
/// monitor and no global rates (so the list is exactly se's).
#[test]
fn virtual_display_windows_task_settings_cover_menu() {
    for choice in SIZE_CHOICES {
        assert!(MODE_SIZES.contains(&choice), "{choice:?} missing");
    }
    let xml = settings_xml(&MODE_SIZES, &REFRESH_RATES);
    assert!(xml.contains("<count>1</count>"));
    assert!(!xml.contains("<g_refresh_rate>"));
    assert_eq!(
        xml.matches("<resolution>").count(),
        MODE_SIZES.len() * REFRESH_RATES.len()
    );
    assert!(xml.contains(
        "<width>2992</width>\n            <height>1344</height>\n            <refresh_rate>240</refresh_rate>"
    ));
    let mut unique = MODE_SIZES.to_vec();
    unique.sort_by_key(|size| (size.width, size.height));
    unique.dedup();
    assert_eq!(unique.len(), MODE_SIZES.len());
}

/// A listed size is taken exactly at its highest rate; a size between two
/// listed ones gets the largest listed one within it with the closest aspect
/// ratio; a size below every listed one the smallest one.
#[test]
fn virtual_display_windows_task_best_mode() {
    let modes = listed(&MODE_SIZES);
    let exact = best_mode(size(2560, 1440), &modes);
    assert_eq!(
        exact,
        Some(ListedMode {
            size: size(2560, 1440),
            refresh_hz: 240
        })
    );
    // A viewer window of 1900 × 1000: 1600 × 900 (16:9) fits; 1680 × 1050 no.
    assert_eq!(
        best_mode(size(1900, 1000), &modes).map(|mode| mode.size),
        Some(size(1600, 900))
    );
    // Equal area: 1920 × 1200 wins over 1200 × 1920 for a wide window.
    assert_eq!(
        best_mode(
            size(3000, 2000),
            &listed(&[size(1920, 1200), size(1200, 1920)])
        )
        .map(|mode| mode.size),
        Some(size(1920, 1200))
    );
    assert_eq!(
        best_mode(size(640, 360), &modes).map(|mode| mode.size),
        Some(size(800, 600))
    );
    assert_eq!(best_mode(size(1920, 1080), &[]), None);
}

fn place(device: &str, x: i32, y: i32, width: u32, height: u32, primary: bool) -> Placement {
    Placement {
        device: device.into(),
        x,
        y,
        width,
        height,
        primary,
        refresh_hz: 0,
    }
}

/// The virtual monitor becomes the primary at 0,0; the physical monitors
/// keep their arrangement to its right, its own former entry is dropped.
#[test]
fn virtual_display_windows_task_layout() {
    let original = vec![
        place(r"\\.\DISPLAY1", 0, 0, 2560, 1440, true),
        place(r"\\.\DISPLAY2", -1920, 200, 1920, 1080, false),
        place(r"\\.\DISPLAY5", 2560, 0, 800, 600, false),
    ];
    let layout = layout_with_virtual(&original, r"\\.\DISPLAY5", size(1920, 1200));
    assert_eq!(
        layout,
        vec![
            place(r"\\.\DISPLAY5", 0, 0, 1920, 1200, true),
            place(r"\\.\DISPLAY1", 1920 + 1920, 0, 2560, 1440, false),
            place(r"\\.\DISPLAY2", 1920, 200, 1920, 1080, false),
        ]
    );
    // Headless: the virtual monitor alone.
    assert_eq!(
        layout_with_virtual(&[], r"\\.\DISPLAY3", size(1280, 720)),
        vec![place(r"\\.\DISPLAY3", 0, 0, 1280, 720, true)]
    );
}

fn adapter(device_id: &str, description: &str, monitor: bool) -> AdapterFact {
    AdapterFact {
        device_id: device_id.into(),
        description: description.into(),
        attached: monitor,
        monitor,
    }
}

/// se's adapter, the fallback adapter, Hyper-V and RDP never count as a
/// physical monitor; a GPU output with a monitor does.
#[test]
fn virtual_display_windows_task_physical_monitor() {
    assert!(adapter_is_ours(r"ROOT\MttVDD", ""));
    assert!(adapter_is_ours(r"root\mttvdd", ""));
    assert!(adapter_is_ours("", ADAPTER_DESCRIPTION));
    let headless = [
        adapter(r"Root\MttVDD", ADAPTER_DESCRIPTION, true),
        adapter(
            r"ROOT\BasicDisplay",
            "Microsoft Basic Display Adapter",
            true,
        ),
        adapter(r"VMBUS\{da0a7802}", "Microsoft Hyper-V Video", true),
        adapter(
            r"RdpIdd_IndirectDisplay",
            "Microsoft Remote Display Adapter",
            true,
        ),
        adapter(r"PCI\VEN_10DE&DEV_2684", "NVIDIA GeForce RTX 4090", false),
    ];
    assert!(!physical_monitor(&headless));
    let mut desk = headless.to_vec();
    desk.push(adapter(
        r"PCI\VEN_10DE&DEV_2684",
        "NVIDIA GeForce RTX 4090",
        true,
    ));
    assert!(physical_monitor(&desk));
}

/// Same driver files and a working device: nothing, no UAC prompt; a broken
/// device: install again without a new signature; other files: sign.
/// The driver hash ignores the order and the case of the names.
#[test]
fn virtual_display_windows_task_driver_step() {
    let pinned = driver_hash(&[
        ("MttVDD.dll", "aa"),
        ("mttvdd.cat", "bb"),
        ("MttVDD.inf", "cc"),
    ]);
    assert_eq!(
        pinned,
        driver_hash(&[
            ("mttvdd.inf", "cc"),
            ("MTTVDD.CAT", "bb"),
            ("mttvdd.dll", "aa")
        ])
    );
    assert_ne!(
        pinned,
        driver_hash(&[
            ("MttVDD.dll", "ab"),
            ("mttvdd.cat", "bb"),
            ("MttVDD.inf", "cc")
        ])
    );
    let state = InstalledState {
        driver_hash: pinned.clone(),
        release: "25.7.23".into(),
        certificate: "0123".into(),
        oem_inf: "oem7.inf".into(),
    };
    let facts = |device_ok, package_ok, settings_ok| DriverFacts {
        device_ok,
        package_ok,
        settings_ok,
    };
    let fine = facts(true, true, true);
    assert_eq!(
        driver_step(Some(&state), &pinned, fine),
        DriverStep::Nothing
    );
    // A broken device or a new mode list: install the signed package again.
    assert_eq!(
        driver_step(Some(&state), &pinned, facts(false, true, true)),
        DriverStep::Repair
    );
    assert_eq!(
        driver_step(Some(&state), &pinned, facts(true, true, false)),
        DriverStep::Repair
    );
    // The signed package is gone: only a new signature helps.
    assert_eq!(
        driver_step(Some(&state), &pinned, facts(false, false, true)),
        DriverStep::SignAndInstall
    );
    assert_eq!(
        driver_step(Some(&state), "other", fine),
        DriverStep::SignAndInstall
    );
    assert_eq!(driver_step(None, &pinned, fine), DriverStep::SignAndInstall);
    let unsigned = InstalledState {
        certificate: String::new(),
        ..state
    };
    assert_eq!(
        driver_step(Some(&unsigned), &pinned, fine),
        DriverStep::SignAndInstall
    );
    // Updates: nothing to do never asks; a declined offer is not repeated
    // unless se installed the driver before.
    assert!(!update_offers_step(DriverStep::Nothing, true, false));
    assert!(update_offers_step(DriverStep::Repair, true, true));
    assert!(update_offers_step(DriverStep::SignAndInstall, false, false));
    assert!(!update_offers_step(DriverStep::SignAndInstall, false, true));
}

/// User rule 2026-11-24 (install at use time): a missing or broken driver is
/// installed when a viewer asks, also after a declined update offer; one
/// install runs at a time and a later request after a decline or failure
/// starts again; the end names its cause in one statistics-safe line; every
/// state has its German text.
#[test]
fn virtual_display_windows_task_install_on_use_rules() {
    assert!(installs_on_use(Some(REASON_DRIVER_MISSING)));
    assert!(installs_on_use(Some(REASON_DRIVER_BROKEN)));
    for other in [Some(REASON_MONITOR_MISSING), Some("x11_modes"), None] {
        assert!(!installs_on_use(other), "{other:?}");
    }
    // The decline marker governs only the unsolicited offer of an update.
    assert!(!update_offers_step(DriverStep::SignAndInstall, false, true));
    let mut board = OnDemand::new();
    assert_eq!(board.reason(), None);
    assert!(board.request(true), "the first request starts the install");
    assert_eq!(board.reason(), Some(REASON_DRIVER_INSTALL_PROMPT));
    assert!(!board.request(false), "a second request waits for it");
    assert_eq!(board.reason(), Some(REASON_DRIVER_INSTALL_PROMPT));
    board.finished();
    assert_eq!(board.reason(), None);
    assert!(board.request(false), "a later request starts again");
    assert_eq!(board.reason(), Some(REASON_DRIVER_INSTALLING));
    // How it ended: only a line written this time names the cause.
    let failed = |text: &str| InstallEnd::Failed(text.into());
    let note = "virtueller Bildschirm: Treiber laden: HTTP 404; erneut\n";
    for (code, note, ended) in [
        (Ok(0), Some("stale"), InstallEnd::Done),
        (Ok(EXIT_DECLINED), None, InstallEnd::Declined),
        (
            Ok(55),
            Some(note),
            failed("Treiber laden: HTTP 404; erneut"),
        ),
        (Ok(1), None, failed("der Schritt endete mit 1")),
        (
            Ok(EXIT_PROMPT_FAILED),
            Some("stale"),
            failed("Windows konnte die Administrator-Bestätigung nicht öffnen"),
        ),
        (
            Err("keine Erhöhung: x".to_string()),
            None,
            failed("keine Erhöhung: x"),
        ),
    ] {
        assert_eq!(InstallEnd::from_exit(code, note), ended);
    }
    assert_eq!(InstallEnd::Done.reason(), None);
    let declined = InstallEnd::Declined.reason().unwrap();
    assert_eq!(declined, REASON_DRIVER_DECLINED);
    let reason = failed("Treiber laden: HTTP 404; erneut").reason().unwrap();
    assert_eq!(
        reason,
        "windows_display_driver_install_failed: Treiber laden: HTTP 404, erneut"
    );
    let long = failed(&format!("a\nb{}", "x".repeat(300)))
        .reason()
        .unwrap();
    assert!(long.starts_with("windows_display_driver_install_failed: a b"));
    assert!(long.ends_with('…') && long.matches('x').count() == 237);
    // The texts of the viewers and of `se screen probe`.
    let text = |state, reason: &str| {
        let report = VirtualDisplayReport::new(state, Some(size(1920, 1080)), "vdd");
        report.with_reason(reason)
    };
    let prompt = text(VirtualDisplayState::Setup, REASON_DRIVER_INSTALL_PROMPT);
    assert_eq!(
        prompt.chain_key(),
        "setup:windows_display_driver_install_prompt"
    );
    assert!(prompt.text().starts_with(
        "Virtueller Bildschirm: Treiber wird installiert – Windows fragt auf dem Gastgeber nach \
         Administrator-Bestätigung; bis dahin zeigt die Sitzung den bisherigen Monitor"
    ));
    assert_eq!(prompt.limit(), None, "no error while it installs");
    let linux = VirtualDisplayReport::new(VirtualDisplayState::Setup, None, "dummy");
    assert_eq!(linux.chain_key(), "setup");
    let unsupported = |reason: &str| text(VirtualDisplayState::Unsupported, reason).text();
    assert!(unsupported(&declined).contains("wurde auf dem Gastgeber abgelehnt"));
    let limit = text(VirtualDisplayState::Unsupported, &declined).limit();
    assert_eq!(
        limit.unwrap(),
        format!("virtual_display_unsupported:{declined}")
    );
    assert!(unsupported(&reason).contains("fehl: Treiber laden: HTTP 404, erneut; eine erneute"));
    assert_eq!(
        unsupported(REASON_DRIVER_MISSING),
        "Virtueller Bildschirm: nicht möglich (der Treiber des virtuellen Bildschirms fehlt)"
    );
    assert_eq!(reason_text("x11_without_display_modes"), None);
    // The statistics keys in German (`chain_text`), other states as they are.
    for (key, german) in [
        (
            "setup:windows_display_driver_installing",
            "Treiber wird installiert; bis dahin",
        ),
        (
            "unsupported:windows_display_driver_declined;source=monitor:2",
            "nicht möglich (die Admin",
        ),
        ("setup", "wird eingerichtet (Freigabe des Betriebssystems)"),
        ("unsupported:x11_modes", "nicht möglich (x11_modes)"),
    ] {
        assert!(chain_text(key).unwrap().starts_with(german), "{key}");
    }
    for other in ["active:1920x1080:vdd", "inactive:off", "offered:1x1:portal"] {
        assert_eq!(chain_text(other), None, "{other}");
    }
}

/// A Windows host whose driver is missing, as `win_display` probes it; its
/// setup is the install at use time with its one-at-a-time board.
struct Missing {
    board: Mutex<OnDemand>,
    starts: AtomicUsize,
}

impl VirtualDisplayDriver for Missing {
    fn probe(&self) -> io::Result<DisplayProbe> {
        Ok(DisplayProbe {
            physical_monitor: true,
            modes: false,
            limit: DriverLimit::NONE,
            driver: "vdd".into(),
            current: Some(size(1920, 1080)),
            unsupported: Some(REASON_DRIVER_MISSING.into()),
            setup_possible: installs_on_use(Some(REASON_DRIVER_MISSING)),
            portal: false,
        })
    }
    fn apply(&self, size: FrameSize) -> io::Result<FrameSize> {
        Err(io::Error::other(format!("no driver for {size:?}")))
    }
    fn request_setup(&self) -> SetupProgress {
        if self.board.lock().unwrap().request(true) {
            self.starts.fetch_add(1, Ordering::SeqCst);
        }
        SetupProgress::Running
    }
    fn setup_adds_display(&self) -> bool {
        true
    }
    fn setup_reason(&self) -> Option<String> {
        self.board.lock().unwrap().reason().map(str::to_string)
    }
}

/// The sessions around an install at use time (Windows beside a physical
/// monitor, the case of 2026-11-24): the probe offers the display with an
/// install instead of `unsupported`; a viewer with „Bildschirm steuern“
/// starts it and sees the German notice while the session keeps its
/// monitor; a second one joins it; a view-only viewer gets the exact reason;
/// a decline reaches the waiting sessions once and a later request starts
/// again; after success each waiting session decides again once, without a
/// new install from that decision.
#[test]
fn virtual_display_windows_task_install_on_use_sessions() {
    let driver = Arc::new(Missing {
        board: Mutex::new(OnDemand::new()),
        starts: AtomicUsize::new(0),
    });
    let dyn_driver: Arc<dyn VirtualDisplayDriver> = driver.clone();
    let probe = capability(Some(dyn_driver.clone())).unwrap();
    assert_eq!(probe.chain_key(), "inactive:setup_needed");
    assert_eq!(probe.limit(), None);
    let start = |session: &str, wish, may_setup| SessionStart {
        session: session.into(),
        capture: format!("{session}-capture"),
        mode: SessionMode::View,
        metadata_only: false,
        wish: Some(wish),
        fixed: None,
        display_size: Some(size(2560, 1440)),
        may_setup,
        monitor: Some("DISPLAY1".into()),
    };
    let open = |session: &str, wish, may_setup| {
        let started = host::start(Some(dyn_driver.clone()), &start(session, wish, may_setup));
        let report = started.report.clone().unwrap();
        // The host state is process-wide: a dropped claim could give the
        // display of `p9_task_switch…` back while it runs.
        std::mem::forget(started);
        report
    };
    let starts = || driver.starts.load(Ordering::SeqCst);
    let a = open("vd-use-a", VirtualDisplayWish::Request, true);
    assert_eq!(a.chain_key(), "setup:windows_display_driver_install_prompt");
    assert!(a.live && a.text().contains("Administrator-Bestätigung"));
    assert_eq!(starts(), 1);
    let running = capability(Some(dyn_driver.clone())).unwrap();
    assert_eq!(running.chain_key(), a.chain_key());
    let b = open("vd-use-b", VirtualDisplayWish::Request, true);
    assert_eq!(b.chain_key(), a.chain_key());
    assert_eq!(starts(), 1, "the second request joins the running install");
    let view = open("vd-use-view", VirtualDisplayWish::Request, false);
    assert_eq!(view.chain_key(), "unsupported:setup_needs_control");
    assert!(view.text().contains("„Bildschirm steuern“"));
    let auto = open("vd-use-auto", VirtualDisplayWish::Auto, true);
    assert_eq!(auto.chain_key(), "inactive:physical_monitor");
    assert_eq!(starts(), 1);
    // Declined: the waiting sessions name it once, nothing decides again.
    driver.board.lock().unwrap().finished();
    host::setup_ended(SetupProgress::Failed(REASON_DRIVER_DECLINED.into()));
    for session in ["vd-use-a", "vd-use-b"] {
        assert_eq!(host::resume_due(session), None);
        assert_eq!(
            host::chain_key(session).as_deref(),
            Some("unsupported:windows_display_driver_declined")
        );
    }
    assert_eq!(host::resume_due("vd-use-view"), None);
    // A later request tries again and keeps the session's monitor meanwhile.
    let request = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Request,
        size: None,
    };
    let again = host::switch("vd-use-a", request, "vd-use-a-2").unwrap();
    assert_eq!(again.report.chain_key(), a.chain_key());
    assert_eq!(again.shown(), Shown::Physical);
    assert_eq!(starts(), 2);
    // Done: the session decides again once with its choice.
    driver.board.lock().unwrap().finished();
    host::setup_ended(SetupProgress::Done);
    assert_eq!(host::resume_due("vd-use-a"), Some(request));
    assert_eq!(host::resume_due("vd-use-a"), None, "once");
    assert_eq!(host::resume_due("vd-use-b"), None, "b no longer waited");
    // That decision starts no install: this fake's driver is still missing.
    let resumed = host::switch("vd-use-a", request, "vd-use-a-3").unwrap();
    assert_eq!(
        resumed.report.chain_key(),
        "unsupported:windows_display_driver_missing"
    );
    assert_eq!(starts(), 2);
    // The viewer's next request starts it again.
    let next = host::switch("vd-use-a", request, "vd-use-a-4").unwrap();
    assert_eq!(next.report.chain_key(), a.chain_key());
    assert_eq!(starts(), 3);
}
