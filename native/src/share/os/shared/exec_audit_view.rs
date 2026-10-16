//! Readable entries of the Exec audit log for the desktop GUI and the Android
//! app (`se share exec log` prints the raw JSON lines).
//!
//! The files are private to this installation, but their text came from
//! other devices and a file can be edited from outside: every field goes
//! through the same cut and character replacement as when it was written
//! (`exec_audit_text::limited`), and a line that is no JSON object is shown as
//! "unreadable" instead of being dropped.
use serde::Serialize;
use serde_json::Value;

use super::text::limited;

/// Most entries one view reads (newest first). Both files of both classes
/// hold at most 16 MiB together; a screen shows a few hundred lines.
pub(crate) const MAX_VIEW_ENTRIES: usize = 1000;

/// One audit line as the GUIs show it; every text field is cut and cleaned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecAuditEntry {
    /// RFC 3339 time of the entry, empty when the line has none.
    pub at: String,
    /// `auth_rejected`, `penalty_refused`, `start_refused`, `started`,
    /// `finished`, another recorded name, or `unreadable`.
    pub event: String,
    /// German label of `event`.
    pub title: String,
    /// Device name (and id) of an authorized device, the claimed device of
    /// a refused authentication, or the EndpointId of a penalized peer.
    pub device: String,
    /// Device id (claimed device id for refused authentications); empty when
    /// unknown. Used to filter the view by device.
    pub device_id: String,
    /// One line with the remaining fields of the entry.
    pub detail: String,
}

/// The entry of one stored line.
pub(super) fn entry(line: &str) -> ExecAuditEntry {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(line) else {
        return ExecAuditEntry {
            at: String::new(),
            event: "unreadable".into(),
            title: "Unlesbare Zeile".into(),
            device: String::new(),
            device_id: String::new(),
            detail: limited(line),
        };
    };
    let value = Value::Object(fields);
    let text = |key: &str| field(&value, key);
    let event = text("event");
    let (title, device, device_id, detail) = match event.as_str() {
        "auth_rejected" => (
            "Anmeldung abgelehnt",
            or_unknown(text("claimed_device")),
            text("claimed_device"),
            join(&[
                labelled("Grund", text("reason")),
                labelled("EndpointId", text("remote_node")),
            ]),
        ),
        "penalty_refused" => (
            "Abgewiesen (Strafkonto)",
            or_unknown(text("remote_node")),
            String::new(),
            join(&[
                labelled("Protokoll", text("protocol")),
                labelled("gesperrt noch (s)", text("remaining_secs")),
            ]),
        ),
        "start_refused" => (
            "Start abgelehnt",
            principal(&value),
            text("device_id"),
            join(&[start_summary(&value), labelled("Grund", text("reason"))]),
        ),
        "started" => (
            "Gestartet",
            principal(&value),
            text("device_id"),
            join(&[
                start_summary(&value),
                labelled("Konto", text("account")),
                labelled("Programmdatei", text("resolved_program")),
                labelled("Verzeichnis", text("cwd")),
            ]),
        ),
        "finished" => (
            "Beendet",
            principal(&value),
            text("device_id"),
            finished_summary(&value),
        ),
        _ => (
            "Eintrag",
            principal(&value),
            text("device_id"),
            limited(line),
        ),
    };
    ExecAuditEntry {
        at: text("at"),
        event: if event.is_empty() {
            "unreadable".into()
        } else {
            event
        },
        title: title.into(),
        device,
        device_id,
        detail,
    }
}

/// Newest first: the log keeps the oldest first.
pub(super) fn entries(lines: Vec<String>) -> Vec<ExecAuditEntry> {
    lines.iter().rev().map(|line| entry(line)).collect()
}

/// A field as cleaned text: strings and numbers, empty when absent.
fn field(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::String(text) => limited(text),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

fn principal(value: &Value) -> String {
    let name = field(value, "device_name");
    let id = field(value, "device_id");
    match (name.is_empty(), id.is_empty()) {
        (false, false) => format!("{name} ({id})"),
        (false, true) => name,
        (true, false) => id,
        (true, true) => "unbekanntes Geraet".into(),
    }
}

fn start_summary(value: &Value) -> String {
    let mode = match field(value, "mode").as_str() {
        "argv" => "Programm".to_string(),
        "shell" => "Shell-Befehl".to_string(),
        "terminal" => "Terminal".to_string(),
        other => other.to_string(),
    };
    join(&[
        labelled("Art", mode),
        labelled("Programm", field(value, "program")),
        labelled("Digest", short_digest(field(value, "digest"))),
    ])
}

fn finished_summary(value: &Value) -> String {
    join(&[
        labelled("Programm", field(value, "program")),
        labelled("Ergebnis", field(value, "result")),
        labelled("Exit", field(value, "exit_code")),
        labelled("Signal", field(value, "signal")),
        labelled("Dauer (s)", field(value, "duration_secs")),
        labelled("stdout (B)", field(value, "stdout_bytes")),
        labelled("stderr (B)", field(value, "stderr_bytes")),
    ])
}

fn short_digest(digest: String) -> String {
    digest.chars().take(16).collect()
}

fn labelled(label: &str, value: String) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("{label}: {value}")
    }
}

fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn or_unknown(text: String) -> String {
    if text.is_empty() {
        "unbekannt".into()
    } else {
        text
    }
}
