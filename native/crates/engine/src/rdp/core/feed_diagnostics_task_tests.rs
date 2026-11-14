//! Tests of the RDP feed diagnostics (E54, S16); `types.rs` includes this file.
use super::{HostProbe, RdpFeedSummary, SetupState};

fn summary_with(diagnostics: Vec<String>) -> RdpFeedSummary {
    let mut probe = HostProbe::unavailable("linux", "test");
    probe.diagnostics = diagnostics;
    RdpFeedSummary::from_probe(&probe, SetupState::NotNeeded)
}

#[test]
fn rdp_diag_task_feed_diagnostics_serialized_with_field() {
    let summary = summary_with(vec!["GNOME: Netz nicht erreichbar".to_string()]);
    let json = serde_json::to_value(&summary).expect("feed summary serializes");
    assert_eq!(
        json["diagnostics"],
        serde_json::json!(["GNOME: Netz nicht erreichbar"])
    );
}

#[test]
fn rdp_diag_task_feed_diagnostics_missing_field_is_empty() {
    // An older sender omits the field; the reader then sees no lines.
    let older: RdpFeedSummary =
        serde_json::from_str(r#"{"startable":true,"setup":"done","remote_apps":[]}"#)
            .expect("older feed summary parses");
    assert!(older.diagnostics.is_empty());
}

#[test]
fn rdp_diag_task_feed_diagnostics_present_field_is_read() {
    let newer: RdpFeedSummary = serde_json::from_str(
        r#"{"startable":false,"setup":"not_needed","remote_apps":[],"diagnostics":["erste","zweite"]}"#,
    )
    .expect("feed summary with the field parses");
    assert_eq!(
        newer.diagnostics,
        vec!["erste".to_string(), "zweite".to_string()]
    );
}

#[test]
fn rdp_diag_task_feed_diagnostics_bounded_to_16_lines_of_300_chars() {
    let lines: Vec<String> = (0..20)
        .map(|index| format!("{index:03}{}", "x".repeat(400)))
        .collect();
    let summary = summary_with(lines);
    assert_eq!(summary.diagnostics.len(), 16);
    assert!(summary
        .diagnostics
        .iter()
        .all(|line| line.chars().count() == 300));
    assert!(summary.diagnostics[15].starts_with("015"));

    // A peer that ignores the bounds (S16): the reader keeps the first 16
    // lines, each cut to 300 characters, and skips the rest.
    let sent: Vec<String> = (0..20)
        .map(|index| format!("{index:03}{}", "y".repeat(400)))
        .collect();
    let oversized = serde_json::json!({
        "startable": true,
        "setup": "done",
        "remote_apps": [],
        "diagnostics": sent,
    });
    let read: RdpFeedSummary =
        serde_json::from_value(oversized).expect("oversized feed summary parses");
    assert_eq!(read.diagnostics.len(), 16);
    assert!(read
        .diagnostics
        .iter()
        .all(|line| line.chars().count() == 300));
    assert!(read.diagnostics[15].starts_with("015"));
}

#[test]
fn rdp_diag_task_feed_diagnostics_cut_at_character_boundary() {
    let summary = summary_with(vec!["ä".repeat(301), "kurz".to_string()]);
    assert_eq!(summary.diagnostics[0].chars().count(), 300);
    assert!(summary.diagnostics[0].chars().all(|c| c == 'ä'));
    assert_eq!(summary.diagnostics[1], "kurz");
}
