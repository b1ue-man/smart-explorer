//! Direct-code renewal as `se share status`, `se share identity` and the MCP
//! `status` tool show it (E17, plan M22). Every value comes from the one
//! `CodeRenewalView` of the daemon snapshot (or of the loaded identity), the
//! JSON keys are the keys of that view, so all surfaces read the same state.
use crate::daemon::CodeRenewalView;
use serde_json::{json, Value};

/// `Direct code is being renewed, 2 devices pending (of which 1 on an older version)`.
pub(in crate::cli::share) fn summary(view: &CodeRenewalView) -> String {
    let pending = view.pending_devices;
    let mut text = format!(
        "Direct code is being renewed, {pending} {} pending",
        if pending == 1 { "device" } else { "devices" }
    );
    match view.legacy_devices {
        0 => {}
        1 => text.push_str(" (of which 1 on an older version)"),
        legacy => text.push_str(&format!(" (of which {legacy} on older versions)")),
    }
    text
}

/// Pending devices whose names the view does not list (at most 32 are named).
fn unnamed(view: &CodeRenewalView) -> usize {
    view.pending_devices
        .saturating_sub(view.pending_names.len())
}

/// The `code_renewal` member of the JSON documents: `null` without a
/// renewal, else the fields of the view plus `pending_more` and `summary`.
pub(in crate::cli::share) fn value(view: Option<&CodeRenewalView>) -> Value {
    let Some(view) = view else {
        return Value::Null;
    };
    json!({
        "active": view.active,
        "generation": view.generation,
        "pending_devices": view.pending_devices,
        "legacy_devices": view.legacy_devices,
        "pending_names": view.pending_names,
        "pending_more": unnamed(view),
        "since": view.since,
        "deferred": view.deferred,
        "error": view.error,
        "summary": view.active.then(|| summary(view)),
    })
}

/// Tab-separated lines for the text outputs; empty without a renewal.
pub(in crate::cli::share) fn text_lines(view: Option<&CodeRenewalView>) -> Vec<String> {
    let Some(view) = view else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if view.active {
        lines.push(format!(
            "code_renewal\tactive\tpending={}\tlegacy={}\tgeneration={}\t{}",
            view.pending_devices,
            view.legacy_devices,
            view.generation,
            summary(view)
        ));
        for name in &view.pending_names {
            lines.push(format!("code_renewal_pending\t{}", clean(name)));
        }
        let more = unnamed(view);
        if more > 0 {
            lines.push(format!("code_renewal_more\t{more}"));
        }
    }
    if view.deferred {
        lines.push(
            "code_renewal_deferred\tAnother renewal waits until a running transition ends".into(),
        );
    }
    if let Some(error) = &view.error {
        lines.push(format!("code_renewal_error\t{}", clean(error)));
    }
    lines
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "code_renewal_output_task_tests.rs"]
mod code_renewal_output_task_tests;
