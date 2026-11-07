//! Direct-code renewal in `ShareStatus` (E17, plan M22): the same
//! `CodeRenewalView` of the worker snapshot that `se share status`, the
//! desktop pages and the MCP status show, as `codeRenewal` for the app. The
//! app shows `summary` and `namesLine` as they are.
use serde_json::{json, Value};

use crate::daemon::CodeRenewalView;

/// „Code wird erneuert, 2 Geräte stehen aus (davon 1 mit älterer Version)“.
pub(super) fn summary(view: &CodeRenewalView) -> String {
    let pending = view.pending_devices;
    let mut text = if pending == 1 {
        "Code wird erneuert, 1 Gerät steht aus".to_string()
    } else {
        format!("Code wird erneuert, {pending} Geräte stehen aus")
    };
    match view.legacy_devices {
        0 => {}
        1 => text.push_str(" (davon 1 mit älterer Version)"),
        legacy => text.push_str(&format!(" (davon {legacy} mit älteren Versionen)")),
    }
    text
}

/// Pending devices whose names the view does not list (at most 32 are named).
fn unnamed(view: &CodeRenewalView) -> usize {
    view.pending_devices
        .saturating_sub(view.pending_names.len())
}

/// „Ausstehend: A, B und 3 weitere“.
fn names_line(view: &CodeRenewalView) -> Option<String> {
    if !view.active || view.pending_names.is_empty() {
        return None;
    }
    let mut line = format!("Ausstehend: {}", view.pending_names.join(", "));
    let more = unnamed(view);
    if more > 0 {
        line.push_str(&format!(" und {more} weitere"));
    }
    Some(line)
}

/// „Fehler bei der Code-Erneuerung: …“.
fn error_text(error: &str) -> String {
    format!("Fehler bei der Code-Erneuerung: {error}")
}

/// The `codeRenewal` member of `ShareStatus`: `null` without a renewal.
pub(super) fn value(view: Option<&CodeRenewalView>) -> Value {
    let Some(view) = view else {
        return Value::Null;
    };
    json!({
        "active": view.active,
        "generation": view.generation,
        "pendingDevices": view.pending_devices,
        "legacyDevices": view.legacy_devices,
        "pendingNames": view.pending_names,
        "pendingMore": unnamed(view),
        "sinceMs": view.since.map(|since| since.saturating_mul(1000)),
        "deferred": view.deferred,
        "error": view.error,
        "errorText": view.error.as_ref().map(|error| error_text(error)),
        "summary": view.active.then(|| summary(view)),
        "namesLine": names_line(view),
        "deferredText": view.deferred.then_some(
            "Eine weitere Erneuerung wartet, bis ein laufender Übergang endet."
        ),
    })
}

/// Whether the renewal generation differs from the previous snapshot's: the
/// Direct code of the cached identity may be outdated then.
pub(super) fn changed(now: Option<&CodeRenewalView>, before: Option<&CodeRenewalView>) -> bool {
    now.map(|view| view.generation) != before.map(|view| view.generation)
}

#[cfg(test)]
#[path = "share_code_renewal_task_tests.rs"]
mod task_tests;
