//! The texts of a failed update (field failure 2026-11-17: `se update --check`
//! showed only `runtime_sha256_mismatch`, the app nothing).
use super::failure::{explain, next_action, reason, PendingUpdate};

fn roll_back() -> PendingUpdate {
    PendingUpdate::RollBack {
        previous_version: "0.5.197".into(),
        target_version: "0.5.198".into(),
    }
}

#[test]
fn update_sha_task_field_failure_names_reason_and_the_rollback_of_the_next_run() {
    let failure = explain(Some("runtime_sha256_mismatch\n"), Some(roll_back()), true).unwrap();
    assert_eq!(failure.error.as_deref(), Some("runtime_sha256_mismatch"));
    assert!(
        failure.reason.contains("SHA-256-Prüfsumme"),
        "{}",
        failure.reason
    );
    assert!(failure.reason.contains("v0.5.198"), "{}", failure.reason);
    let next = &failure.next_action;
    assert!(
        next.contains("setzt die unterbrochene Aktualisierung auf v0.5.197 zurück und installiert danach v0.5.198"),
        "{next}"
    );
    assert!(
        next.contains("`se update`") && next.contains("MCP-Server"),
        "{next}"
    );
    assert_eq!(failure.pending, Some(roll_back()));
}

#[test]
fn update_sha_task_reason_follows_the_most_specific_code() {
    let rollback = "Frühere Aktualisierung abschließen: update_rollback_failed: lock_busy: x";
    assert!(reason(rollback).starts_with("Eine unterbrochene frühere Aktualisierung"));
    let refused = "Frühere Aktualisierung abschließen: update_recovery_refused: se.exe ist v1";
    assert!(reason(refused).contains("gemischt"));
    let source = "Pruefsumme passt nicht fuer C:\\x: erwartet a, erhalten b";
    assert!(reason(source).contains("Update-Quelle"));
    assert!(reason("lock_busy: control.lock").contains("länger als 5 Sekunden"));
    assert!(reason("etwas Neues").contains("Meldung des Updaters steht unverändert dabei"));
}

#[test]
fn update_sha_task_nothing_to_show_without_report_or_while_an_update_runs() {
    let running = PendingUpdate::Running {
        target_version: "0.5.198".into(),
    };
    assert_eq!(explain(None, None, true), None);
    assert_eq!(explain(Some("  \n"), None, true), None);
    assert_eq!(explain(None, Some(running), true), None);
    let unfinished = explain(None, Some(roll_back()), false).unwrap();
    assert_eq!(unfinished.error, None);
    assert!(
        unfinished.reason.contains("v0.5.198 wurde unterbrochen"),
        "{}",
        unfinished.reason
    );
    assert!(
        unfinished.next_action.contains("v0.5.197 zurück"),
        "{}",
        unfinished.next_action
    );
}

#[test]
fn update_sha_task_next_action_names_each_resolution_and_the_startup_check() {
    let complete = PendingUpdate::Complete {
        previous_version: "0.5.197".into(),
        target_version: "0.5.198".into(),
    };
    assert!(next_action(Some(&complete), true).contains("schon eingespielte v0.5.198"));
    let refused = PendingUpdate::Refused {
        previous_version: "0.5.197".into(),
        target_version: "0.5.198".into(),
        detail: "se.exe ist v0.5.198".into(),
    };
    let text = next_action(Some(&refused), true);
    assert!(text.starts_with("Kein automatischer Weg") && text.contains("se.exe ist v0.5.198"));
    assert!(next_action(None, true).contains("beim nächsten Start"));
    assert!(next_action(None, false).contains("Suche beim Start aus ist"));
    let json = serde_json::to_value(roll_back()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"resolution": "roll_back", "previous_version": "0.5.197", "target_version": "0.5.198"})
    );
}
