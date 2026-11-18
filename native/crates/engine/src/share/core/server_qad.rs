//! The Share server's self-check of its QUIC address discovery (QAD, UDP
//! 7842) as this device last heard it (`qad_status_v1`, TODO
//! `DIREKT-INTERNET`). The server judges from the devices that reach it
//! whether QAD arrives from outside (`share-server/src/qad_reach.rs`); a
//! blocked UDP 7842 leaves devices behind NAT without a public candidate, so
//! connections across the internet run over the relay. `se share status`,
//! the desktop and Android show the verdict.
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// Hello capability: the server sends its verdict in `hello_ok` and as
/// `qad_status`.
pub const QAD_STATUS_CAPABILITY: &str = "qad_status_v1";

/// The state of one address family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QadReach {
    Reachable,
    Unreachable,
    #[default]
    Unknown,
    /// The server's QAD does not listen for this family.
    Off,
}

impl QadReach {
    /// A word of a newer server reads as unknown, not as an error.
    fn from_wire(word: &str) -> Self {
        match word {
            "reachable" => Self::Reachable,
            "unreachable" => Self::Unreachable,
            "off" => Self::Off,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Reachable => "erreichbar",
            Self::Unreachable => "nicht erreichbar",
            Self::Unknown => "unbekannt",
            Self::Off => "aus",
        }
    }
}

/// The server's verdict per address family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerQadStatus {
    pub ipv4: QadReach,
    pub ipv6: QadReach,
}

/// The verdict as the server sends it; unknown words stay readable.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ServerQadWire {
    #[serde(default)]
    ipv4: String,
    #[serde(default)]
    ipv6: String,
}

impl From<&ServerQadWire> for ServerQadStatus {
    fn from(wire: &ServerQadWire) -> Self {
        Self {
            ipv4: QadReach::from_wire(&wire.ipv4),
            ipv6: QadReach::from_wire(&wire.ipv6),
        }
    }
}

impl ServerQadStatus {
    /// The `se share status` line: „Server: QAD von außen erreichbar/nicht
    /// erreichbar (UDP 7842)“ with both families.
    pub fn status_line(&self) -> String {
        let families = format!(
            "UDP 7842; IPv4: {}, IPv6: {}",
            self.ipv4.label(),
            self.ipv6.label()
        );
        let any = |reach: QadReach| self.ipv4 == reach || self.ipv6 == reach;
        if any(QadReach::Unreachable) {
            format!(
                "Server: QAD von außen nicht erreichbar ({families}) – ohne QAD keine öffentliche \
                 Adresse, Verbindungen über das Internet laufen über das Relay; UDP 7842 in der \
                 Firewall des Servers öffnen"
            )
        } else if any(QadReach::Reachable) {
            format!("Server: QAD von außen erreichbar ({families})")
        } else if self.ipv4 == QadReach::Off && self.ipv6 == QadReach::Off {
            format!("Server: QAD aus ({families}; der Server bietet keine Adresserkennung an)")
        } else {
            format!(
                "Server: QAD von außen noch nicht geprüft ({families}; noch kein Gerät von außen \
                 verbunden)"
            )
        }
    }

    /// The machine-readable form (`se share status --json`, Android).
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "ipv4": self.ipv4,
            "ipv6": self.ipv6,
            "line": self.status_line(),
        })
    }
}

/// What the current server said; `None` before its `hello_ok` or when it
/// predates the check.
static LAST: Mutex<Option<ServerQadStatus>> = Mutex::new(None);

/// Records the verdict of `hello_ok` (`None`: a server without the check)
/// or of a `qad_status` message.
pub(super) fn record(status: Option<ServerQadStatus>) {
    *LAST.lock().unwrap_or_else(PoisonError::into_inner) = status;
}

/// The verdict last received from the server.
pub fn server_qad_status() -> Option<ServerQadStatus> {
    *LAST.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{QadReach, ServerQadStatus, ServerQadWire};

    fn status(ipv4: &str, ipv6: &str) -> ServerQadStatus {
        let wire: ServerQadWire =
            serde_json::from_value(serde_json::json!({ "ipv4": ipv4, "ipv6": ipv6 })).unwrap();
        ServerQadStatus::from(&wire)
    }

    #[test]
    fn direkt_internet_task_server_qad_status_line() {
        let blocked = status("unreachable", "reachable").status_line();
        assert!(blocked.starts_with("Server: QAD von außen nicht erreichbar (UDP 7842"));
        assert!(blocked.contains("IPv4: nicht erreichbar, IPv6: erreichbar"));
        assert_eq!(
            status("reachable", "unknown").status_line(),
            "Server: QAD von außen erreichbar (UDP 7842; IPv4: erreichbar, IPv6: unbekannt)"
        );
        assert!(status("off", "off")
            .status_line()
            .starts_with("Server: QAD aus"));
        assert!(status("unknown", "off")
            .status_line()
            .starts_with("Server: QAD von außen noch nicht geprüft"));
        // A word of a newer server is unknown, not a parse error.
        assert_eq!(status("partly", "").ipv4, QadReach::Unknown);
        assert_eq!(status("partly", "").ipv6, QadReach::Unknown);
    }
}
