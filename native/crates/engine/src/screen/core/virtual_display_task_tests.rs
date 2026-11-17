//! Task tests of the virtual display (plan 2026-11-17 P5): the fitting to a
//! driver's limit, CVT reduced blanking as `cvt -r` prints it, which session
//! decides and the facts of a physical monitor. The contracts (Xorg files,
//! hello fields, elevated order) are in `virtual_display_contract_task_tests.rs`.
//! No test needs an X server.
use std::io;
use std::sync::Arc;

use super::*;
use crate::screen::core::virtual_display_config::VIDEO_RAM_KIB;
use crate::screen::core::virtual_display_host::{
    capability, DisplayProbe, SetupProgress, VirtualDisplayDriver,
};
use crate::screen::core::wire_hello::SessionMode;

const MIB: u64 = 1024 * 1024;

fn size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width, height)
}

/// bochs-drm under `modesetting` with QEMU's 16 MiB: a mode fits into half
/// of the video memory, so 1920 × 1080 is the largest 16:9 display; larger
/// wishes keep their aspect ratio within it.
#[test]
fn virtual_display_task_fit_bochs_16mib() {
    let bochs = DriverLimit::bochs(16 * MIB);
    assert_eq!(bochs.max_pixels, 2_097_152);
    assert_eq!(fit_mode(size(1920, 1080), bochs), Some(size(1920, 1080)));
    assert_eq!(fit_mode(size(1024, 768), bochs), Some(size(1024, 768)));
    // 2560 × 1440 and 3840 × 2160: the widest multiple of 8 within the
    // memory, the height by the aspect ratio.
    assert_eq!(fit_mode(size(2560, 1440), bochs), Some(size(1928, 1084)));
    assert_eq!(fit_mode(size(3840, 2160), bochs), Some(size(1928, 1084)));
    // The X server's size range narrows it further.
    let vga = DriverLimit::sides(1920, 1200).narrowed(bochs);
    assert_eq!(fit_mode(size(2560, 1600), vga), Some(size(1824, 1140)));
    // The Xorg driver `dummy` with se's 1 GiB takes what se's capture takes.
    let dummy = DriverLimit::dummy(VIDEO_RAM_KIB);
    assert_eq!(dummy.max_pixels, 268_435_456);
    assert_eq!(fit_mode(size(3840, 2160), dummy), Some(size(3840, 2160)));
}

/// E2: the width is a multiple of 8 with the aspect ratio kept, a tiny
/// window is enlarged to 640 × 360, every side stays within 16384 and the
/// pixels within what se's capture takes (`FrameLimits::RESOURCE`).
#[test]
fn virtual_display_task_fit_alignment_and_cap() {
    let none = DriverLimit::NONE;
    assert_eq!(none.max_width, 16_384);
    assert_eq!(none.max_height, 16_384);
    assert_eq!(none.max_pixels, 16_777_216);
    assert_eq!(fit_mode(size(1366, 768), none), Some(size(1360, 764)));
    assert_eq!(fit_mode(size(1921, 1081), none), Some(size(1920, 1080)));
    assert_eq!(fit_mode(size(320, 200), none), Some(size(640, 400)));
    assert_eq!(fit_mode(size(100, 400), none), Some(size(640, 2560)));
    assert_eq!(fit_mode(size(20_000, 10_000), none), Some(size(5792, 2896)));
    assert_eq!(fit_mode(size(16_384, 100), none), Some(size(16_384, 100)));
    assert_eq!(fit_mode(size(0, 1080), none), None);
    assert_eq!(fit_mode(size(1920, 0), none), None);
    for (width, height) in [(1366, 768), (2561, 1439), (999, 333), (7680, 4320)] {
        let fitted = fit_mode(size(width, height), none).unwrap();
        assert_eq!(fitted.width % WIDTH_STEP, 0, "{fitted:?}");
        assert!(
            fitted.width <= 16_384 && fitted.height <= 16_384,
            "{fitted:?}"
        );
        assert!(fitted.pixels() <= none.max_pixels, "{fitted:?}");
    }
    // A driver below the minimum keeps what it can.
    let tiny = DriverLimit::sides(320, 240);
    assert_eq!(fit_mode(size(1920, 1080), tiny), Some(size(320, 180)));
    // Every size of the viewer's list is set exactly.
    for choice in SIZE_CHOICES {
        assert_eq!(fit_mode(choice, none), Some(choice), "{choice:?}");
    }
}

/// CVT reduced blanking as `cvt -r` (libxcvt) prints it, its X11 mode line
/// and se's mode names.
#[test]
fn virtual_display_task_cvt_rb_1920x1080() {
    let timing = cvt_rb(1920, 1080, 60).unwrap();
    assert_eq!(
        timing,
        ModeTiming {
            clock_khz: 138_500,
            hdisplay: 1920,
            hsync_start: 1968,
            hsync_end: 2000,
            htotal: 2080,
            vdisplay: 1080,
            vsync_start: 1083,
            vsync_end: 1088,
            vtotal: 1111,
        }
    );
    assert_eq!(
        timing.modeline("SE_1920x1080"),
        "Modeline \"SE_1920x1080\" 138.50 1920 1968 2000 2080 1080 1083 1088 1111 +hsync -vsync"
    );
    // `cvt -r` of further sizes: 16:9, 16:10 (vsync 6), 4:3 (vsync 4) and a
    // custom ratio (vsync 10).
    let line = |width, height| {
        let timing = cvt_rb(width, height, 60).unwrap();
        timing.modeline("m")
    };
    assert_eq!(
        line(2560, 1440),
        "Modeline \"m\" 241.50 2560 2608 2640 2720 1440 1443 1448 1481 +hsync -vsync"
    );
    assert_eq!(
        line(3840, 2160),
        "Modeline \"m\" 533.00 3840 3888 3920 4000 2160 2163 2168 2222 +hsync -vsync"
    );
    assert_eq!(
        line(1280, 800),
        "Modeline \"m\" 71.00 1280 1328 1360 1440 800 803 809 823 +hsync -vsync"
    );
    assert_eq!(
        line(1360, 768),
        "Modeline \"m\" 72.00 1360 1408 1440 1520 768 771 781 790 +hsync -vsync"
    );
    // The minimum vertical blank (3 + 5 + 6 lines) of a short mode.
    assert_eq!(cvt_rb(640, 360, 60).unwrap().vtotal, 374);
    assert!(cvt_rb(0, 1080, 60).is_err());
    assert!(cvt_rb(1920, 0, 60).is_err());
    assert!(cvt_rb(1920, 1080, 0).is_err());
    assert_eq!(mode_name(size(2560, 1440)), "SE_2560x1440");
    assert!(is_own_mode("SE_2560x1440"));
    for other in [
        "1920x1080",
        "SE_",
        "SE_x1080",
        "SE_1920x",
        "SE_19a0x1080",
        "se_1920x1080",
    ] {
        assert!(!is_own_mode(other), "{other}");
    }
}

/// E2: the newest session with control decides, else the newest view
/// session; a resize keeps the order; a fixed size is not resized; after
/// the last session the size stays; at least 1 s between two changes.
#[test]
fn virtual_display_task_owner_rules() {
    let none = DriverLimit::NONE;
    let mut planner = Planner::default();
    assert_eq!(planner.due(none, 0), Due::Idle);
    planner.claim("a", false, size(1280, 720), false);
    planner.claim("b", false, size(1920, 1080), false);
    assert_eq!(planner.target(none), Some(size(1920, 1080)));
    planner.claim("c", true, size(1600, 900), false);
    assert_eq!(
        planner.target(none),
        Some(size(1600, 900)),
        "control decides"
    );
    planner.claim("d", false, size(2560, 1440), false);
    assert_eq!(
        planner.target(none),
        Some(size(1600, 900)),
        "control still decides"
    );
    planner.release("c");
    assert_eq!(
        planner.target(none),
        Some(size(2560, 1440)),
        "the newest view"
    );
    // A resize of an older session does not make it the newest.
    assert!(planner.resize("a", size(3840, 2160)));
    assert_eq!(planner.target(none), Some(size(2560, 1440)));
    assert!(!planner.resize("unknown", size(800, 600)));
    // A fixed size stays.
    planner.claim("e", false, size(1920, 1200), true);
    assert!(!planner.resize("e", size(1000, 700)));
    assert_eq!(planner.target(none), Some(size(1920, 1200)));
    // Timing: now, then nothing, then the gap of 1 s.
    assert_eq!(planner.due(none, 5_000), Due::Now(size(1920, 1200)));
    planner.changed(size(1920, 1200), 5_000);
    assert_eq!(planner.due(none, 5_001), Due::Idle);
    planner.release("e");
    assert_eq!(planner.due(none, 5_400), Due::Later(600));
    assert_eq!(planner.due(none, 6_000), Due::Now(size(2560, 1440)));
    // A failed attempt counts too: the same size is not asked again.
    planner.changed(size(2560, 1440), 6_000);
    assert_eq!(planner.due(none, 9_000), Due::Idle);
    for session in ["a", "b", "d"] {
        planner.release(session);
    }
    assert!(!planner.has_claims());
    assert_eq!(planner.due(none, 20_000), Due::Idle);
    assert_eq!(planner.applied(), Some(size(2560, 1440)), "the size stays");
    // A physical monitor got its own mode back: the next claim applies.
    planner.forget();
    planner.claim("f", false, size(2560, 1440), false);
    assert_eq!(planner.due(none, 21_000), Due::Now(size(2560, 1440)));
    // The deciding claim directly.
    let claims = [
        Claim {
            session: "x".into(),
            control: false,
            order: 2,
            size: size(800, 600),
            fixed: false,
        },
        Claim {
            session: "y".into(),
            control: true,
            order: 1,
            size: size(1024, 768),
            fixed: false,
        },
    ];
    assert_eq!(
        owner(&claims).map(|claim| claim.session.as_str()),
        Some("y")
    );
    assert_eq!(owner(&[]), None);
}

struct Fake(DisplayProbe);

impl VirtualDisplayDriver for Fake {
    fn probe(&self) -> io::Result<DisplayProbe> {
        Ok(self.0.clone())
    }
    fn apply(&self, size: FrameSize) -> io::Result<FrameSize> {
        Ok(size)
    }
    fn request_setup(&self) -> SetupProgress {
        SetupProgress::Running
    }
}

fn probe(
    physical_monitor: bool,
    modes: bool,
    unsupported: Option<&str>,
    setup: bool,
) -> DisplayProbe {
    DisplayProbe {
        physical_monitor,
        modes,
        limit: DriverLimit::NONE,
        driver: "randr".into(),
        current: Some(size(1024, 768)),
        unsupported: unsupported.map(str::to_string),
        setup_possible: setup,
        portal: false,
    }
}

/// E1: what tells a physical monitor, when a session may change the
/// display, when the driver setup may run, and what a probe reports.
#[test]
fn virtual_display_task_needs_virtual_facts() {
    let connector = |kind: &str, connected: bool, edid_bytes: usize| ConnectorFact {
        kind: kind.into(),
        connected,
        edid_bytes,
    };
    let facts = |connectors: Vec<ConnectorFact>, outputs: &[&str]| PhysicalFacts {
        connectors,
        display_outputs: outputs.iter().map(|name| name.to_string()).collect(),
    };
    assert!(facts(vec![connector("HDMI-A", true, 256)], &[]).physical_monitor());
    assert!(!facts(vec![connector("Virtual", true, 128)], &[]).physical_monitor());
    assert!(!facts(vec![connector("Writeback", true, 128)], &[]).physical_monitor());
    assert!(!facts(vec![connector("DP", true, 0)], &[]).physical_monitor());
    assert!(!facts(vec![connector("DP", false, 256)], &[]).physical_monitor());
    // VESA without DRM: no monitor; a driver that hides its connectors
    // from DRM shows them as X outputs.
    assert!(!facts(Vec::new(), &[]).physical_monitor());
    assert!(!facts(Vec::new(), &["default"]).physical_monitor());
    assert!(!facts(Vec::new(), &["DUMMY0", "Virtual-1"]).physical_monitor());
    assert!(facts(Vec::new(), &["DP-0"]).physical_monitor());
    assert!(!facts(vec![connector("Virtual", true, 128)], &["DP-0"]).physical_monitor());
    assert_eq!(connector_kind("card0-HDMI-A-1"), Some("HDMI-A"));
    assert_eq!(connector_kind("card1-Virtual-1"), Some("Virtual"));
    assert_eq!(connector_kind("card0-DP-2"), Some("DP"));
    for name in ["card0", "renderD128", "version", "cardX-DP-1", "card0-DP-x"] {
        assert_eq!(connector_kind(name), None, "{name}");
    }
    for name in ["DUMMY0", "Virtual-1", "Virtual1", "VIRTUAL-1"] {
        assert!(virtual_output_name(name), "{name}");
    }
    for name in ["DP-0", "HDMI-A-1", "default", "eDP-1"] {
        assert!(!virtual_output_name(name), "{name}");
    }
    use VirtualDisplayWish::{Auto, Off, Request};
    for mode in [SessionMode::View, SessionMode::Control] {
        assert!(needs_virtual(mode, false, Some(Auto), false));
        assert!(
            !needs_virtual(mode, false, Some(Auto), true),
            "never at a monitor"
        );
        assert!(needs_virtual(mode, false, Some(Request), true));
        assert!(!needs_virtual(mode, false, Some(Off), false));
        assert!(!needs_virtual(mode, false, None, false), "an older viewer");
        assert!(!needs_virtual(mode, true, Some(Request), false), "metadata");
    }
    for mode in [SessionMode::Probe, SessionMode::Preview] {
        assert!(!needs_virtual(mode, false, Some(Request), false));
    }
    assert!(setup_allowed(Some(Auto), false) && setup_allowed(Some(Request), false));
    assert!(
        !setup_allowed(Some(Request), true),
        "never replaces a monitor"
    );
    assert!(!setup_allowed(Some(Off), false) && !setup_allowed(None, false));
    // The answer of a probe session.
    let report = |probe: DisplayProbe| {
        let driver: Arc<dyn VirtualDisplayDriver> = Arc::new(Fake(probe));
        capability(Some(driver)).unwrap()
    };
    let available = report(probe(false, true, None, false));
    assert_eq!(available.state, VirtualDisplayState::Inactive);
    assert_eq!(available.chain_key(), "inactive:available");
    let monitor = report(probe(true, true, None, false));
    assert_eq!(monitor.chain_key(), "inactive:physical_monitor");
    let setup = report(probe(false, false, Some("x11_without_display_modes"), true));
    assert_eq!(setup.chain_key(), "inactive:setup_needed");
    let pending = report(probe(false, false, Some(WINDOWS_DRIVER_PENDING), false));
    assert_eq!(pending.state, VirtualDisplayState::Unsupported);
    assert_eq!(
        pending.limit().as_deref(),
        Some("virtual_display_unsupported:windows_driver_pending")
    );
    assert!(capability(None).is_none());
}
