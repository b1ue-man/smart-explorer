//! The chain lines of the statistics field (plan 2026-11-14 „Bildkette“ BK0,
//! 4.10a): the stage of every station the host reports (capture path with
//! its fallback, conversion on GPU or CPU, read-backs, pointer in the
//! picture, a capped capture rate), what limits frame rate and bit rate,
//! the rate the encoder is given and whether it keeps it (plan 2026-11-17),
//! „Übertragung“ and the rate the host measured on the line (user
//! 2026-11-25, `screen_delivery.rs`), the kinds of recovery, the host's station times (after `FRAME_TIMING`,
//! the network with the clock offset) and the viewer's own stations
//! (decode, hand-over to the graphics, dropped pictures, pictures drawn
//! again because no new one came). A host before
//! BK0 reports none of it: its lines say „nicht gemeldet“. The desktop
//! viewer and the remote window show the same lines (`view_lines`).
use super::screen_session::SessionInfo;
use crate::screen::core::frame_timing::{ChainReport, StationUs};

const NOT_REPORTED: &str = "nicht gemeldet";

fn limit_text(key: &str) -> &str {
    match key {
        "viewer" => "Wunsch des Betrachters",
        "source" => "Quelle",
        "encode_time" => "Kodierzeit",
        "link" => "Leitung",
        // The viewer's own „Datenrate“ (formerly „Höchste Datenrate“) and,
        // under „Automatisch“, the rate the host measured (user 2026-11-25).
        "ceiling" => "Datenrate wie gewählt",
        "measured" => "Datenrate wie gemessen",
        "codec_ceiling" => "Obergrenze der Codec-Paarung",
        "ramp" => "Anlauf",
        "content" => "Bildinhalt",
        "encoder_ceiling" => "Obergrenze des Kodierers",
        "encoder_ignores" => "Kodierer hält die Rate nicht",
        other => other,
    }
}

/// The verdict of the host's honour meter (plan 2026-11-17).
fn honour_text(key: &str) -> &str {
    match key {
        "pending" => "noch nicht beurteilt",
        "honoured" => "eingehalten",
        "tolerated" => "knapp eingehalten",
        "corrected" => "korrigiert",
        "ignores" => "nicht eingehalten",
        other => other,
    }
}

/// The holding station of `value`, „nicht gemeldet“ without one.
fn limit(value: &Option<String>) -> &str {
    value.as_deref().map_or(NOT_REPORTED, limit_text)
}

fn path_text(key: &str) -> &str {
    match key {
        "desktop_duplication" => "Desktop Duplication",
        "windows_graphics_capture" => "Windows Graphics Capture",
        "pipewire_dmabuf" => "PipeWire DMA-BUF",
        "pipewire_shm" => "PipeWire SHM",
        "xshm" => "XShm",
        "x11_get_image" => "X11 GetImage",
        "media_projection" => "MediaProjection",
        "test_picture" => "Testbild",
        other => other,
    }
}

fn ms(station: StationUs) -> String {
    format!(
        "p50 {:.1} ms, p95 {:.1} ms",
        station.p50_us as f64 / 1000.0,
        station.p95_us as f64 / 1000.0
    )
}

fn window_ms(percentiles: Option<(f64, f64)>) -> String {
    percentiles.map_or_else(
        || NOT_REPORTED.to_string(),
        |(p50, p95)| format!("p50 {p50:.1} ms, p95 {p95:.1} ms"),
    )
}

/// The stage and limit lines of `chain`.
fn stage_lines(chain: &ChainReport) -> Vec<String> {
    let capture = match (&chain.capture_path, &chain.capture_fallback) {
        (Some(path), Some(cause)) => format!("{} (Rückfall: {cause})", path_text(path)),
        (Some(path), None) => path_text(path).to_string(),
        (None, _) => NOT_REPORTED.to_string(),
    };
    let convert = match chain.convert.as_deref() {
        Some("gpu") => "GPU".to_string(),
        Some("cpu") => "CPU".to_string(),
        Some(other) => other.to_string(),
        None => NOT_REPORTED.to_string(),
    };
    let yes_no = |value: Option<bool>| match value {
        Some(true) => "ja",
        Some(false) => "nein",
        None => NOT_REPORTED,
    };
    let count = |value: Option<u64>| value.map_or(NOT_REPORTED.to_string(), |n| n.to_string());
    let fps = |value: Option<u32>| value.map_or("–".to_string(), |n| n.to_string());
    vec![
        format!("Aufnahme {capture}"),
        format!(
            "Umwandlung {convert}, Rücklesungen {}",
            count(chain.readbacks)
        ),
        format!(
            "Zeiger im Bild {}, Aufnahmerate begrenzt {}",
            yes_no(chain.cursor_embedded),
            yes_no(chain.capture_rate_capped)
        ),
        format!(
            "Bildrate {} von {} B/s, begrenzt durch {}",
            fps(chain.target_fps),
            fps(chain.wanted_fps),
            limit(&chain.fps_limit)
        ),
        format!("Datenrate begrenzt durch {}", limit(&chain.rate_limit)),
        match (chain.encoder_rate_kbps, chain.rate_honour.as_deref()) {
            (Some(kbps), Some(honour)) => {
                format!("Kodierer-Rate {kbps} kbit/s ({})", honour_text(honour))
            }
            (Some(kbps), None) => format!("Kodierer-Rate {kbps} kbit/s"),
            (None, _) => format!("Kodierer-Rate {NOT_REPORTED}"),
        },
        format!(
            "Erholungen: Fortsetzung {}, Schlüsselbild {}",
            count(chain.recoveries_continued),
            count(chain.recoveries_keyframe)
        ),
    ]
}

/// The chain lines of the field.
pub fn chain_lines(info: &SessionInfo) -> Vec<String> {
    let stats = info.stats.as_ref();
    let mut lines = match stats.and_then(|stats| stats.chain.as_ref()) {
        Some(chain) => {
            let mut lines = stage_lines(chain);
            lines.extend(super::screen_delivery::stats_lines(chain));
            lines
        }
        None => vec![format!("Bildkette: {NOT_REPORTED} (ältere Version)")],
    };
    match stats.and_then(|stats| stats.stations.as_ref()) {
        Some(stations) => {
            let named = [
                ("Takt-Abweichung", stations.tick_jitter),
                ("Aufnahme", stations.capture),
                ("Umwandlung", stations.convert),
                ("Kodierung", stations.encode),
                ("Übergabe Hostprozess", stations.worker_handoff),
                ("Senden", stations.send),
            ];
            for (name, station) in named {
                let value = station.map_or_else(|| NOT_REPORTED.to_string(), ms);
                lines.push(format!("Gastgeber {name} {value}"));
            }
            let network = stations
                .network_raw
                .map(|raw| ms(raw.shifted(info.clock_offset_us)));
            lines.push(format!(
                "Netz {}",
                network.unwrap_or_else(|| NOT_REPORTED.to_string())
            ));
        }
        None => lines.push(format!("Stationszeiten des Gastgebers: {NOT_REPORTED}")),
    }
    lines.push(format!(
        "Dekodierung {}",
        window_ms(info.latency.decode.percentiles_ms())
    ));
    lines.push(format!(
        "Übergabe an die Anzeige {}",
        window_ms(info.latency.handover.percentiles_ms())
    ));
    lines.push(format!(
        "Verworfen beim Betrachter {}",
        info.latency.dropped
    ));
    lines.push(format!("Wiederholt gezeichnet {}", info.latency.repeated));
    lines
}

#[cfg(test)]
#[path = "screen_stats_chain_tests.rs"]
mod tests;
