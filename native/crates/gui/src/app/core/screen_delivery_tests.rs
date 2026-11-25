//! „Übertragung“ and the measured „Datenrate“ of the desktop viewer (user
//! 2026-11-25): the stored choice next to an older viewer's file, the
//! statistics lines and the toolbar's texts from the host's report.
use std::time::Duration;

use super::super::screen_prefs::ViewerPrefs;
use super::super::screen_session::{SessionInfo, Shared};
use super::*;
use crate::screen::core::wire_control::HostStats;

fn info(chain: Option<ChainReport>) -> SessionInfo {
    let mut info = Shared::new(false, 0).info().clone();
    info.stats = Some(HostStats {
        chain,
        ..HostStats::default()
    });
    info
}

fn reported(delivery: Delivery, kbps: Option<u64>, age_ms: Option<u64>) -> ChainReport {
    ChainReport {
        delivery: Some(delivery),
        measured_kbps: kbps,
        measured_age_ms: age_ms,
        ..ChainReport::default()
    }
}

/// A file of a viewer before „Übertragung“ reads as „Echtzeit“; the choice
/// is written under its key, read back alike and kept when another setting
/// is saved; never chosen is not written.
#[test]
fn delivery_task_viewer_pref_reads_old_files_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("viewer.json");
    std::fs::write(&path, br#"{"schema":1,"codec":"hevc","audio":false}"#).unwrap();
    let older = ViewerPrefs::load_from(&path);
    assert_eq!(older.delivery, None);
    assert_eq!(older.delivery.unwrap_or_default(), Delivery::Realtime);
    assert_eq!(older.audio, Some(false));
    let mut chosen = older.clone();
    chosen.delivery = Some(Delivery::Reliable);
    chosen.save_to(&path).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["delivery"], "reliable");
    assert_eq!(stored["codec"], "hevc");
    assert_eq!(ViewerPrefs::load_from(&path), chosen);
    // The file's key is the key of `--transport` and the MCP tool.
    for mode in Delivery::ALL {
        assert_eq!(serde_json::to_value(mode).unwrap(), mode.key());
        assert_eq!(Delivery::parse(mode.key()), Some(mode));
    }
    let plain = serde_json::to_value(ViewerPrefs::default()).unwrap();
    assert!(plain.get("delivery").is_none(), "{plain}");
    // Another setting saved by a process that loaded the older file.
    let mut other = older.clone();
    other.low_latency = true;
    other.save_changes_to(&older, &path).unwrap();
    let merged = ViewerPrefs::load_from(&path);
    assert_eq!(merged.delivery, Some(Delivery::Reliable));
    assert!(merged.low_latency);
}

/// The statistics name the mode the host runs and the rate it measured,
/// with its age; `rate_limit` `measured` reads „Datenrate wie gemessen“; a
/// host before „Übertragung“ says „nicht gemeldet“.
#[test]
fn delivery_task_statistics_name_transport_and_measured_rate() {
    let chain = reported(Delivery::Reliable, Some(48_200), Some(3_400));
    assert_eq!(
        stats_lines(&chain),
        [
            "Übertragung Zuverlässig",
            "Datenrate gemessen 48,2 Mbit/s (vor 3 s)"
        ]
    );
    let fresh = reported(Delivery::Realtime, None, None);
    assert_eq!(
        stats_lines(&fresh),
        ["Übertragung Echtzeit", "Datenrate noch nicht gemessen"]
    );
    let undated = reported(Delivery::Realtime, Some(1_500), None);
    assert_eq!(stats_lines(&undated)[1], "Datenrate gemessen 1,5 Mbit/s");
    assert_eq!(
        stats_lines(&ChainReport::default()),
        ["Übertragung nicht gemeldet (ältere Version)"]
    );
    let mut limited = chain.clone();
    limited.rate_limit = Some("measured".into());
    let lines = super::super::screen_stats_chain::chain_lines(&info(Some(limited)));
    for expected in [
        "Datenrate begrenzt durch Datenrate wie gemessen",
        "Übertragung Zuverlässig",
        "Datenrate gemessen 48,2 Mbit/s (vor 3 s)",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected}: {lines:#?}"
        );
    }
    assert_eq!(
        automatic_text(measured(&chain), true),
        "Automatisch (gemessen: 48,2 Mbit/s)"
    );
    assert_eq!(
        automatic_text(None, true),
        "Automatisch (noch nicht gemessen)"
    );
    assert_eq!(automatic_text(None, false), "Automatisch");
    assert_eq!(mbit_text(250_000), "250,0");
}

/// The toolbar's „Übertragung“ names what the host runs; „Neu messen“ and
/// a change go out only to a host whose statistics name „Übertragung“, and
/// „misst …“ ends with a measurement younger than the press.
#[test]
fn delivery_task_toolbar_follows_the_hosts_report() {
    let (realtime, reliable) = (Delivery::Realtime, Delivery::Reliable);
    assert_eq!(
        list_text(realtime, None, false, false),
        "Übertragung: Echtzeit (wie UDP)"
    );
    assert_eq!(
        list_text(reliable, Some(realtime), true, false),
        "Übertragung: Zuverlässig (wie TCP) (wird angewendet …)"
    );
    assert_eq!(
        list_text(reliable, Some(realtime), false, false),
        "Übertragung: Zuverlässig (wie TCP) → läuft: Echtzeit"
    );
    assert_eq!(
        list_text(reliable, Some(reliable), false, false),
        "Übertragung: Zuverlässig (wie TCP)"
    );
    assert_eq!(
        list_text(realtime, None, false, true),
        "Übertragung: Echtzeit (wie UDP) → Gerät kennt die Wahl nicht"
    );
    // No statistics yet: nothing is known, nothing goes out.
    let waiting = Shared::new(false, 0).info().clone();
    assert!(!negotiated(&waiting) && !unknown_to_host(&waiting));
    // A host before „Übertragung“ (statistics without it, or before BK0).
    for older in [info(Some(ChainReport::default())), info(None)] {
        assert!(!negotiated(&older) && unknown_to_host(&older));
    }
    let current = info(Some(reported(reliable, Some(20_000), Some(500))));
    assert!(negotiated(&current) && !unknown_to_host(&current));
    assert_eq!(running(&current), Some(reliable));
    let measured = measured_in(&current);
    assert_eq!(
        measured,
        Some(Measured {
            kbps: 20_000,
            age_ms: Some(500)
        })
    );
    // Pressed 2 s ago, measured 500 ms ago: the result is there.
    assert!(!remeasure_pending(Duration::from_secs(2), measured));
    let older = Some(Measured {
        kbps: 20_000,
        age_ms: Some(5_000),
    });
    assert!(remeasure_pending(Duration::from_secs(2), older));
    assert!(remeasure_pending(Duration::from_secs(1), None));
    assert!(!remeasure_pending(REMEASURE_WAIT, None));
}
