//! Why a host rejected a session (R3, plan 2026-11-06 „Zugriffe trotz
//! Freigabe“). The class travels additively in the hello rejection
//! (`FsResponse::Err::denial`); hosts before it send only the message, and
//! clients before it read only the message. The peer already proved its key
//! over TLS before the host classifies, so a class tells it nothing beyond
//! its own relation; a failed proof stays the generic `Authentication`.
use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};

/// Secret-free reason of one rejected session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionDenial {
    /// No Direct grant for this identity on the host (one-sided pairing,
    /// removed grant, new installation of the peer).
    NotGranted,
    /// The host suspended the grant after its own code rotation or identity
    /// repair („neu bestätigen“).
    Reconfirm,
    /// The host's user blocked this device („Sperren“).
    Blocked,
    /// The peer used another Direct code than the host's current one.
    CodeChanged,
    /// The host has Direct access switched off.
    Offline,
    /// The host does not take part in the room (left, not auto-joining).
    RoomUnknown,
    /// The host has not admitted this device to the room (or blocked it).
    RoomAdmission,
    /// The stored identity of the relation does not match.
    Identity,
    /// The session proof failed; deliberately without detail.
    Authentication,
    /// A class of a newer host.
    #[serde(other)]
    Unknown,
}

impl SessionDenial {
    /// Stable code for JSON outputs (CLI, MCP, Android bridge).
    pub fn code(self) -> &'static str {
        match self {
            Self::NotGranted => "not_granted",
            Self::Reconfirm => "reconfirm",
            Self::Blocked => "blocked",
            Self::CodeChanged => "code_changed",
            Self::Offline => "offline",
            Self::RoomUnknown => "room_unknown",
            Self::RoomAdmission => "room_admission",
            Self::Identity => "identity",
            Self::Authentication => "authentication",
            Self::Unknown => "unknown",
        }
    }

    /// The reason, seen from the rejected device.
    pub fn reason(self, host: &str) -> String {
        let host = shown(host);
        match self {
            Self::NotGranted => format!("{host} hat dieses Gerät noch nicht freigegeben"),
            Self::Reconfirm => format!(
                "{host} wartet auf die erneute Bestätigung dieses Geräts (Direkt-Code erneuert oder Identität repariert)"
            ),
            Self::Blocked => format!("{host} hat dieses Gerät gesperrt"),
            Self::CodeChanged => {
                format!("{host} hat einen neuen Direkt-Code; die gespeicherte Kopplung ist veraltet")
            }
            Self::Offline => format!("{host} ist für Direktverbindungen ausgeschaltet"),
            Self::RoomUnknown => format!("{host} ist dem Raum nicht beigetreten"),
            Self::RoomAdmission => format!("{host} hat dieses Gerät im Raum nicht zugelassen"),
            Self::Identity => format!("Die gespeicherte Identität passt nicht zu {host}"),
            Self::Authentication | Self::Unknown => {
                format!("{host} hat die Anmeldung abgelehnt")
            }
        }
    }

    /// What the user of the host (or this device) does to allow access.
    pub fn remedy(self, host: &str) -> String {
        let host = shown(host);
        match self {
            Self::NotGranted => format!(
                "Auf {host} unter „Geräte & Freigaben“ dieses Gerät öffnen und „Eigene Freigaben für dieses Gerät öffnen“ wählen, oder beide Geräte neu koppeln"
            ),
            Self::Reconfirm => format!(
                "Auf {host} unter „Geräte & Freigaben“ „Neu bestätigen“ wählen, oder beide Geräte neu koppeln"
            ),
            Self::Blocked => format!(
                "Auf {host} unter „Geräte & Freigaben“ dieses Gerät öffnen und „Wieder erlauben“ wählen"
            ),
            Self::CodeChanged => format!(
                "Beide Geräte neu koppeln oder den aktuellen Direkt-Code von {host} eingeben"
            ),
            Self::Offline => format!("Auf {host} „Direkt erreichbar“ einschalten"),
            Self::RoomUnknown => {
                format!("Auf {host} dem Raum beitreten oder „Automatisch beitreten“ einschalten")
            }
            Self::RoomAdmission => format!(
                "Auf {host} unter „Geräte & Freigaben“ → Raum → Mitglieder dieses Gerät zulassen"
            ),
            Self::Identity => format!("Das Gerät auf {host} entfernen und beide Geräte neu koppeln"),
            Self::Authentication | Self::Unknown => {
                "Beide Geräte neu koppeln".to_string()
            }
        }
    }

    /// One line with reason and remedy.
    pub fn message(self, host: &str) -> String {
        format!("{}. {}.", self.reason(host), self.remedy(host))
    }
}

fn shown(host: &str) -> &str {
    let host = host.trim();
    if host.is_empty() {
        "Das andere Gerät"
    } else {
        host
    }
}

/// The error a rejected session check returns: the former host-side detail
/// as text (unchanged log lines), the class for the peer.
#[derive(Debug)]
pub struct SessionDenialError {
    pub denial: SessionDenial,
    detail: &'static str,
}

impl fmt::Display for SessionDenialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail)
    }
}

impl std::error::Error for SessionDenialError {}

/// A rejected session check of class `denial` with the host log text `detail`.
pub fn denied(denial: SessionDenial, detail: &'static str) -> io::Error {
    io::Error::other(SessionDenialError { denial, detail })
}

/// The class of a session check error, if it is one.
pub fn denial_of(error: &io::Error) -> Option<SessionDenial> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<SessionDenialError>())
        .map(|denied| denied.denial)
}

/// A rejection a client received: the class (absent from older hosts) and
/// the host's text. Kept inside the `PermissionDenied` error so callers that
/// only read the kind and text behave as before.
#[derive(Debug)]
pub struct PeerDenied {
    pub denial: Option<SessionDenial>,
    pub message: String,
}

impl fmt::Display for PeerDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PeerDenied {}

pub fn peer_denied(denial: Option<SessionDenial>, message: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        PeerDenied { denial, message },
    )
}

/// The error of a rejected hello on the client: a classified host's own
/// reason and way to the grant; for hosts before R3 the generic reply with
/// the ways that can lead to it.
pub fn client_rejection(
    denial: Option<SessionDenial>,
    msg: String,
    host: &super::super::types::PeerPresence,
) -> io::Error {
    if denial.is_some() {
        return peer_denied(denial, msg);
    }
    let host = shown(&host.device_name);
    peer_denied(
        None,
        format!(
            "{host} hat die Anmeldung abgelehnt ({msg}): dieses Gerät ist dort nicht freigegeben, gesperrt oder neu zu bestätigen. Auf {host} unter „Geräte & Freigaben“ dieses Gerät freigeben oder beide Geräte neu koppeln."
        ),
    )
}

/// The class a rejected client session carries.
pub fn peer_denial_of(error: &io::Error) -> Option<Option<SessionDenial>> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<PeerDenied>())
        .map(|denied| denied.denial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechte_task_denial_keeps_detail_and_class() {
        let error = denied(SessionDenial::NotGranted, "Direktfreigabe nicht akzeptiert");
        assert_eq!(error.to_string(), "Direktfreigabe nicht akzeptiert");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(denial_of(&error), Some(SessionDenial::NotGranted));
        assert_eq!(denial_of(&io::Error::other("x")), None);
    }

    #[test]
    fn rechte_task_unknown_class_of_newer_host_parses() {
        let parsed: SessionDenial = serde_json::from_str("\"future_reason\"").unwrap();
        assert_eq!(parsed, SessionDenial::Unknown);
        let encoded = serde_json::to_string(&SessionDenial::Reconfirm).unwrap();
        assert_eq!(encoded, "\"reconfirm\"");
    }

    #[test]
    fn rechte_task_peer_message_names_reason_and_remedy() {
        let text = SessionDenial::NotGranted.message("Laptop");
        assert!(
            text.contains("Laptop hat dieses Gerät noch nicht freigegeben"),
            "{text}"
        );
        assert!(text.contains("Eigene Freigaben"), "{text}");
        let generic = SessionDenial::Authentication.message("");
        assert!(generic.starts_with("Das andere Gerät"), "{generic}");
    }

    #[test]
    fn rechte_task_client_error_stays_permission_denied() {
        let error = peer_denied(Some(SessionDenial::Blocked), "gesperrt".into());
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "gesperrt");
        assert_eq!(peer_denial_of(&error), Some(Some(SessionDenial::Blocked)));
        let legacy = peer_denied(None, "Share-Anmeldung nicht autorisiert".into());
        assert_eq!(peer_denial_of(&legacy), Some(None));
    }
}
