//! The carrier lines of the statistics field (user 2026-11-25: „TCP und
//! UDP, DIREKT“): which way the pictures take, as the host's `STATS` name
//! it (`chain.udp_path`, `chain.tcp_path` with `tcp_link`), why the direct
//! TCP link is missing (`tcp_failure`), and what became of the UDP
//! pictures (`udp_*`). A host before the carriers names none of it: the
//! pictures then take the QUIC streams of the connection's path
//! (`HostStats::path`). The Android viewer shows the same texts
//! (`ScreenViewDelivery.kt`, `ScreenCarrier`).
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::wire_control::HostStats;

/// How the direct TCP link runs (`TcpPathKind::label`).
fn link_text(key: &str) -> &str {
    match key {
        "lan" => "LAN",
        "ipv6" => "IPv6",
        "public" => "öffentlich",
        "punched" => "durch NAT gelocht",
        other => other,
    }
}

/// The way of the pictures: „Bildweg …“.
pub fn path_line(stats: &HostStats) -> String {
    let chain = stats.chain.as_ref();
    let udp = chain.and_then(|chain| chain.udp_path.as_deref());
    let tcp = chain.and_then(|chain| chain.tcp_path.as_deref());
    let link = chain.and_then(|chain| chain.tcp_link.as_deref());
    let way = match (udp, tcp) {
        (Some("direct"), _) => "UDP direkt".to_string(),
        (Some("quic_direct"), _) => "UDP über QUIC (direkt)".to_string(),
        (Some("relay"), _) => "über Relay (QUIC-Datagramme, kein UDP)".to_string(),
        (_, Some("direct")) => match link {
            Some(link) => format!("TCP direkt ({})", link_text(link)),
            None => "TCP direkt".to_string(),
        },
        (_, Some("quic_stream")) => "QUIC-Strom (direkt)".to_string(),
        (_, Some("relay")) => "über Relay (QUIC-Strom)".to_string(),
        (Some(other), _) | (_, Some(other)) => other.to_string(),
        (None, None) => match stats.path {
            PathKind::Direct => "QUIC-Strom (direkt)".to_string(),
            PathKind::Relay => "über Relay (QUIC-Strom)".to_string(),
        },
    };
    format!("Bildweg {way}")
}

/// What became of the UDP pictures; none before the viewer offered UDP.
fn udp_lines(chain: &ChainReport) -> Vec<String> {
    let Some(lost) = chain.udp_lost_pictures else {
        return Vec::new();
    };
    let count = |value: Option<u64>| value.unwrap_or(0);
    vec![
        format!("Verworfen: unvollständig (UDP) {lost}"),
        format!(
            "Verworfen: Kette unterbrochen (UDP) {}",
            count(chain.udp_skipped_pictures)
        ),
        format!(
            "Wiederhergestellt (UDP) {} Bilder, {} von {} verlorenen Paketen",
            count(chain.udp_recovered_pictures),
            count(chain.udp_recovered_packets),
            count(chain.udp_lost_packets)
        ),
    ]
}

/// The carrier lines of the field.
pub fn carrier_lines(stats: &HostStats) -> Vec<String> {
    let mut lines = vec![path_line(stats)];
    if let Some(chain) = stats.chain.as_ref() {
        if let Some(failure) = &chain.tcp_failure {
            lines.push(failure.clone());
        }
        lines.extend(udp_lines(chain));
    }
    lines
}

#[cfg(test)]
#[path = "screen_stats_carrier_tests.rs"]
mod tests;
