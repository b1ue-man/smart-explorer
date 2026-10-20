//! The statistics field of the desktop viewer (SCR-M7, U25-7): codec and
//! encoder, picture size and rate, bit rate, path, round trip, drops, key
//! frames, resets and the latency of the latest pictures in the corner of
//! the picture. A value that is not known shows „–“.
use eframe::egui::{self, Color32, FontId};

use super::screen_session::SessionInfo;

const MISSING: &str = "–";

fn value<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| MISSING.to_string(), |value| value.to_string())
}

/// The lines of the field. `unmapped_keys` counts the keys that had no
/// usage (they were dropped).
pub(crate) fn stats_lines(info: &SessionInfo, unmapped_keys: u64) -> Vec<String> {
    let stream = info.stream.as_ref();
    let stats = info.stats.as_ref();
    let codec = stream.map_or_else(
        || MISSING.to_string(),
        |stream| {
            format!(
                "{}/{}{}",
                stream.codec.key(),
                stream.encoder,
                if stream.hardware { " (Hardware)" } else { "" }
            )
        },
    );
    let size = stream.map_or_else(
        || MISSING.to_string(),
        |stream| format!("{}×{}", stream.size.width, stream.size.height),
    );
    let path = match (info.path.as_deref(), stats) {
        (Some("direct"), _) => "direkt".to_string(),
        (Some("relay"), _) => "Relay".to_string(),
        (Some(other), _) => other.to_string(),
        (None, Some(stats)) if stats.path.key() == "direct" => "direkt".to_string(),
        (None, Some(_)) => "Relay".to_string(),
        (None, None) => MISSING.to_string(),
    };
    let latency = info.latency.percentiles_ms().map_or_else(
        || format!("Latenz p50 {MISSING} ms, p95 {MISSING} ms"),
        |(p50, p95)| format!("Latenz p50 {p50:.0} ms, p95 {p95:.0} ms"),
    );
    let mut lines = vec![
        format!("Codec {codec}"),
        format!("{size} @ {} B/s", value(stats.map(|stats| stats.fps))),
        format!(
            "{} kbit/s (Ziel {})",
            value(stats.map(|stats| stats.sent_kbps)),
            value(stats.map(|stats| stats.target_kbps))
        ),
        format!("Pfad {path}"),
        format!("Umlaufzeit {} ms", value(info.rtt_ms)),
        format!(
            "Verworfen {}",
            value(stats.map(|stats| stats.dropped_before_encode))
        ),
        format!(
            "Schlüsselbilder {}",
            value(stats.map(|stats| stats.keyframes))
        ),
        format!(
            "Rücksetzungen {}",
            value(stats.map(|stats| stats.stream_resets))
        ),
        latency,
    ];
    if unmapped_keys > 0 {
        lines.push(format!("Tasten ohne Abbildung {unmapped_keys}"));
    }
    lines
}

/// Draws the field in the top right corner of `rect`.
pub(crate) fn show(ui: &egui::Ui, rect: egui::Rect, lines: &[String]) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout(
        lines.join("\n"),
        FontId::monospace(12.0),
        Color32::WHITE,
        rect.width(),
    );
    let size = galley.size() + egui::vec2(12.0, 8.0);
    let top_left = rect.right_top() + egui::vec2(-size.x - 8.0, 8.0);
    painter.rect_filled(
        egui::Rect::from_min_size(top_left, size),
        4.0,
        Color32::from_black_alpha(170),
    );
    painter.galley(top_left + egui::vec2(6.0, 4.0), galley, Color32::WHITE);
}
