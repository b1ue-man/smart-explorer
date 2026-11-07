//! R3 (plan 2026-11-06): rejected sessions in both directions as part of
//! the route snapshot, which reaches the desktop, the CLI (`se share status
//! --json`), MCP and the Android bridge without a new IPC field. Each notice
//! names the device, the reason and the way to allow access; incoming
//! rejections that the current rights already allow are left out.
use serde::{Deserialize, Serialize};

use super::super::session::denial::SessionDenial;
use super::super::session::peer_denials::{self, DenialRow};
use super::super::types::ShareAuthState;

/// Newest notices a snapshot carries (bounded IPC budget).
const MAX_NOTICES: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccessNotice {
    /// `outgoing`: the other device rejected this one. `incoming`: this
    /// device rejected the other one.
    pub direction: String,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// This device's contact of the other device, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    /// Stable reason code (`SessionDenial::code`); `unclassified` for hosts
    /// before R3.
    pub reason: String,
    /// The reason as one sentence.
    pub message: String,
    /// The way to allow access.
    pub remedy: String,
    /// Rejections counted (outgoing: in a row).
    pub count: u64,
    /// Unix seconds of the latest rejection.
    pub last_at: i64,
    /// Seconds until this device tries again (outgoing only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_in_secs: Option<u64>,
}

/// The notices of `state`'s device, newest first.
pub(super) fn notices(state: &ShareAuthState) -> Vec<AccessNotice> {
    peer_denials::rows(&state.identity.node_id)
        .into_iter()
        .filter(|row| row.direction == "outgoing" || !now_allowed(state, row))
        .take(MAX_NOTICES)
        .map(|row| notice(state, row))
        .collect()
}

fn now_allowed(state: &ShareAuthState, row: &DenialRow) -> bool {
    let key = row.public_key.as_deref().unwrap_or_default();
    match row.relation_kind.as_deref() {
        Some("direct") => state
            .direct_grants
            .iter()
            .any(|grant| grant.authorizes_session(&row.device_id, key, &row.node_id)),
        Some("room") => state.rooms.iter().any(|room| {
            room.auto_join
                && room
                    .members
                    .iter()
                    .any(|member| member.authorizes_session(&row.device_id, key, &row.node_id))
        }),
        _ => false,
    }
}

fn notice(state: &ShareAuthState, row: DenialRow) -> AccessNotice {
    // R-ZUGANG-03 c: a contact and a name only from a relation that pins the
    // TLS-bound node of the same device; a claimed device id names nobody.
    let contact = peer_denials::pinned_contact(state, &row.device_id, &row.node_id);
    let known_name = peer_denials::known_name(state, &row.device_id, &row.node_id);
    let device_name = row.device_name.clone().or(known_name);
    let shown = device_name.clone().unwrap_or_else(|| {
        if row.direction == "incoming" {
            unknown_device(&row.node_id)
        } else {
            "Das andere Gerät".into()
        }
    });
    let (message, remedy) = if row.direction == "outgoing" {
        outgoing_text(row.denial, row.message.as_deref(), &shown)
    } else {
        incoming_text(row.denial.unwrap_or(SessionDenial::Unknown), &shown)
    };
    AccessNotice {
        direction: row.direction.to_string(),
        device_id: row.device_id,
        device_name,
        contact_id: contact.map(|contact| contact.id.clone()),
        reason: row
            .denial
            .map_or("unclassified", SessionDenial::code)
            .to_string(),
        message,
        remedy,
        count: row.count,
        last_at: row.last_unix,
        retry_in_secs: row.retry_in_secs.filter(|secs| *secs > 0),
    }
}

/// An incoming device no relation of this host pins, named by the start of
/// its TLS-bound node id.
fn unknown_device(node_id: &str) -> String {
    let short: String = node_id.chars().take(8).collect();
    if short.is_empty() {
        "Unbekanntes Gerät".into()
    } else {
        format!("Unbekanntes Gerät ({short}…)")
    }
}

fn outgoing_text(
    denial: Option<SessionDenial>,
    host_text: Option<&str>,
    host: &str,
) -> (String, String) {
    match denial {
        Some(denial) => (denial.reason(host), denial.remedy(host)),
        None => (
            host_text
                .map(str::to_string)
                .unwrap_or_else(|| format!("{host} hat die Anmeldung abgelehnt")),
            format!(
                "Auf {host} unter „Geräte & Freigaben“ dieses Gerät freigeben oder beide Geräte neu koppeln"
            ),
        ),
    }
}

/// Text on the rejecting device (this host) about the device it rejected.
fn incoming_text(denial: SessionDenial, device: &str) -> (String, String) {
    match denial {
        SessionDenial::NotGranted => (
            format!("{device} möchte auf dieses Gerät zugreifen, hat hier aber keine Freigabe"),
            format!("Unter „Geräte & Freigaben“ {device} öffnen und „Eigene Freigaben für dieses Gerät öffnen“ wählen"),
        ),
        SessionDenial::Reconfirm => (
            format!("{device} möchte zugreifen; seine Freigabe wartet auf die erneute Bestätigung"),
            "Unter „Geräte & Freigaben“ „Neu bestätigen“ wählen".to_string(),
        ),
        SessionDenial::Blocked => (
            format!("{device} möchte zugreifen, ist hier aber gesperrt"),
            format!("Unter „Geräte & Freigaben“ {device} öffnen und „Wieder erlauben“ wählen"),
        ),
        SessionDenial::CodeChanged => (
            format!("{device} meldet sich mit einem älteren Direkt-Code dieses Geräts an"),
            "Beide Geräte neu koppeln".to_string(),
        ),
        SessionDenial::Offline => (
            format!("{device} möchte zugreifen, Direktverbindungen sind hier ausgeschaltet"),
            "„Direkt erreichbar“ einschalten".to_string(),
        ),
        SessionDenial::RoomUnknown => (
            format!("{device} möchte über einen Raum zugreifen, dem dieses Gerät nicht beigetreten ist"),
            "Dem Raum beitreten oder „Automatisch beitreten“ einschalten".to_string(),
        ),
        SessionDenial::RoomAdmission => (
            format!("{device} möchte über einen Raum zugreifen, ist dort aber nicht zugelassen"),
            "Unter „Geräte & Freigaben“ den Raum öffnen und das Gerät unter Mitglieder zulassen".to_string(),
        ),
        SessionDenial::Identity | SessionDenial::Authentication | SessionDenial::Unknown => (
            format!("Die Anmeldung von {device} passte nicht zu seiner gespeicherten Identität"),
            format!("{device} entfernen und beide Geräte neu koppeln"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechte_task_incoming_text_names_device_and_action() {
        let (message, remedy) = incoming_text(SessionDenial::NotGranted, "Handy");
        assert!(message.starts_with("Handy möchte"), "{message}");
        assert!(remedy.contains("Eigene Freigaben"), "{remedy}");
        let (message, _) = outgoing_text(None, None, "PC");
        assert_eq!(message, "PC hat die Anmeldung abgelehnt");
    }
}

#[cfg(test)]
#[path = "service_access_notices_task_tests.rs"]
mod task_tests;
