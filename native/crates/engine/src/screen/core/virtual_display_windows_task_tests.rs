//! Task tests of the Windows virtual display rules (plan 2026-11-17 P5,
//! `virtual_display_windows.rs`): the driver's mode list, the listed mode
//! for a wish, the desktop layout with the virtual monitor as primary, what
//! counts as a physical monitor and when the elevated step signs again.
use super::windows_driver::*;
use super::SIZE_CHOICES;
use crate::screen::core::pixel_convert::FrameSize;

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
    // Equal area: 1920 × 1200 (16:10) wins over 1200 × 1920 for 16:10.
    assert_eq!(
        best_mode(
            size(2000, 2000),
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
