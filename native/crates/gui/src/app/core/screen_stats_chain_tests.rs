//! BK0 (plan 2026-11-14 „Bildkette“): the chain lines of the statistics
//! field with a host before BK0 and with a host that reports every value.
use super::chain_lines;
use crate::screen::core::frame_timing::{ChainReport, HostStations, SkewedUs, StationUs};
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::wire_control::HostStats;

fn stats(chain: Option<ChainReport>, stations: Option<HostStations>) -> HostStats {
    HostStats {
        target_kbps: 6000,
        sent_kbps: 5800,
        fps: 60,
        path: PathKind::Direct,
        keyframes: 1,
        dropped_before_encode: 0,
        stream_resets: 0,
        unacked_bytes: 0,
        frame_timing: chain.as_ref().map(|_| 1),
        chain,
        stations,
    }
}

fn info(stats: HostStats) -> super::super::screen_session::SessionInfo {
    let shared = super::super::screen_session::Shared::new(false, 0);
    let mut info = shared.info().clone();
    info.stats = Some(stats);
    info
}

#[test]
fn bildkette_task_stats_viewer_lines_say_not_reported_for_an_old_host() {
    let lines = chain_lines(&info(stats(None, None)));
    assert!(lines.contains(&"Bildkette: nicht gemeldet (ältere Version)".to_string()));
    assert!(lines.contains(&"Stationszeiten des Gastgebers: nicht gemeldet".to_string()));
    assert!(lines.contains(&"Dekodierung nicht gemeldet".to_string()));
    assert!(lines.contains(&"Verworfen beim Betrachter 0".to_string()));
}

#[test]
fn bildkette_task_stats_viewer_lines_name_every_station() {
    let chain = ChainReport {
        capture_path: Some("windows_graphics_capture".into()),
        capture_fallback: Some("dxgi_unsupported".into()),
        convert: Some("gpu".into()),
        capture_rate_capped: Some(true),
        cursor_embedded: Some(false),
        readbacks: Some(0),
        recoveries_continued: Some(2),
        recoveries_keyframe: Some(1),
        target_fps: Some(60),
        wanted_fps: Some(120),
        encode_capacity_fps: Some(60),
        fps_limit: Some("encode_time".into()),
        rate_limit: Some("link".into()),
        rate_ceiling_kbps: None,
        encoder_rate_kbps: Some(12_000),
        rate_honour: Some("corrected".into()),
        virtual_display: None,
        // „Übertragung“ and the measured rate: `screen_delivery_tests.rs`.
        measured_kbps: None,
        measured_age_ms: None,
        delivery: None,
    };
    let station = Some(StationUs {
        p50_us: 1_500,
        p95_us: 2_500,
    });
    let stations = HostStations {
        capture: station,
        convert: None,
        encode: station,
        worker_handoff: station,
        send: station,
        tick_jitter: station,
        input_inject: None,
        network_raw: Some(SkewedUs {
            p50_us: 1_500,
            p95_us: 2_500,
        }),
        input_raw: None,
    };
    let mut info = info(stats(Some(chain), Some(stations)));
    info.clock_offset_us = 0;
    info.latency.decode.record(3_000);
    info.latency.on_put(false);
    info.latency.on_put(true);
    info.latency.on_taken();
    let lines = chain_lines(&info);
    for expected in [
        "Aufnahme Windows Graphics Capture (Rückfall: dxgi_unsupported)",
        "Umwandlung GPU, Rücklesungen 0",
        "Zeiger im Bild nein, Aufnahmerate begrenzt ja",
        "Bildrate 60 von 120 B/s, begrenzt durch Kodierzeit",
        "Datenrate begrenzt durch Leitung",
        "Kodierer-Rate 12000 kbit/s (korrigiert)",
        "Erholungen: Fortsetzung 2, Schlüsselbild 1",
        "Gastgeber Aufnahme p50 1.5 ms, p95 2.5 ms",
        "Gastgeber Umwandlung nicht gemeldet",
        "Gastgeber Übergabe Hostprozess p50 1.5 ms, p95 2.5 ms",
        "Netz p50 1.5 ms, p95 2.5 ms",
        "Dekodierung p50 3.0 ms, p95 3.0 ms",
        "Verworfen beim Betrachter 1",
    ] {
        assert!(
            lines.contains(&expected.to_string()),
            "{expected}: {lines:#?}"
        );
    }
    assert!(lines
        .iter()
        .any(|line| line.starts_with("Übergabe an die Anzeige p50")));
}

/// r1: a paint that showed the previous picture again (no new one came)
/// counts as „wiederholt gezeichnet“; a taken new picture does not.
#[test]
fn bildkette_task_viewer_counts_repeated_pictures() {
    let mut info = info(stats(None, None));
    assert!(chain_lines(&info).contains(&"Wiederholt gezeichnet 0".to_string()));
    info.latency.on_put(false);
    info.latency.on_taken();
    info.latency.on_repeated();
    info.latency.on_repeated();
    assert_eq!(info.latency.repeated, 2);
    assert_eq!(info.latency.dropped, 0, "a repeat is no dropped picture");
    let lines = chain_lines(&info);
    let dropped = lines
        .iter()
        .position(|line| line.starts_with("Verworfen beim Betrachter"))
        .unwrap();
    assert_eq!(lines[dropped + 1], "Wiederholt gezeichnet 2");
    // The session counts what the drawing thread reports.
    let shared = super::super::screen_session::Shared::new(false, 0);
    shared.update(|info| info.latency.on_repeated());
    assert_eq!(shared.info().latency.repeated, 1);
}
