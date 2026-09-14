//! Platform-neutral contract for sharing the internet uplink with a
//! router-less link, plus the durable record of an active session so a
//! restarted daemon can reconcile (stop) what it left behind.
use serde::{Deserialize, Serialize};

use super::link_facts::InterfaceFacts;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Facility {
    Available,
    Unavailable(String),
}

impl Facility {
    pub fn is_available(&self) -> bool {
        matches!(self, Facility::Available)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Facility::Available => None,
            Facility::Unavailable(reason) => Some(reason),
        }
    }
}

/// One interface as the adapter addresses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UplinkTarget {
    pub index: u32,
    pub name: String,
    pub adapter_id: String,
}

impl UplinkTarget {
    pub fn from_facts(facts: &InterfaceFacts) -> Self {
        Self {
            index: facts.index,
            name: facts.name.clone(),
            adapter_id: facts.adapter_id.clone(),
        }
    }
}

/// What the daemon needs from a platform to share an uplink. Every method
/// reports failure explicitly; nothing is assumed to exist.
pub trait UplinkAdapter: Send {
    /// Whether sharing can work here at all (service present, tooling found,
    /// one-time setup done). Cheap enough to call every few seconds.
    fn probe(&mut self, setup_done: bool) -> Facility;

    /// The one-time privileged preparation (UAC/polkit). Returns a message
    /// for the user on success.
    fn setup_once(&mut self) -> Result<String, String>;

    fn enable(&mut self, private: &UplinkTarget, public: &UplinkTarget) -> Result<(), String>;

    fn disable(&mut self, private: &UplinkTarget, public: &UplinkTarget) -> Result<(), String>;

    /// `Some(true)` when the platform confirms sharing is active on the
    /// private link, `None` when it cannot tell.
    fn sharing_active(&mut self, private: &UplinkTarget) -> Result<Option<bool>, String>;

    /// Interfaces the platform confirms as internet-connected, or `None`
    /// when it offers no verdict (a default gateway then counts).
    fn internet_ifaces(&mut self, facts: &[InterfaceFacts]) -> Option<Vec<u32>>;

    /// Fill in DHCP lease knowledge the platform has (NetworkManager).
    fn refine_facts(&mut self, _facts: &mut [InterfaceFacts]) {}
}

/// Adapter for platforms without an implementation.
pub struct UnsupportedAdapter(pub String);

impl UplinkAdapter for UnsupportedAdapter {
    fn probe(&mut self, _setup_done: bool) -> Facility {
        Facility::Unavailable(self.0.clone())
    }

    fn setup_once(&mut self) -> Result<String, String> {
        Err(self.0.clone())
    }

    fn enable(&mut self, _private: &UplinkTarget, _public: &UplinkTarget) -> Result<(), String> {
        Err(self.0.clone())
    }

    fn disable(&mut self, _private: &UplinkTarget, _public: &UplinkTarget) -> Result<(), String> {
        Err(self.0.clone())
    }

    fn sharing_active(&mut self, _private: &UplinkTarget) -> Result<Option<bool>, String> {
        Ok(None)
    }

    fn internet_ifaces(&mut self, _facts: &[InterfaceFacts]) -> Option<Vec<u32>> {
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingRecord {
    pub private_index: u32,
    pub private_name: String,
    pub private_id: String,
    pub public_index: u32,
    pub public_name: String,
    pub public_id: String,
    pub since: i64,
}

/// Durable uplink-sharing state (`lan_uplink_state.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UplinkState {
    #[serde(default)]
    pub sharing: Option<SharingRecord>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl UplinkState {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(text).map_err(|error| format!("Uplink-Status lesen: {error}"))
    }

    pub fn encode(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|error| format!("Uplink-Status kodieren: {error}"))
    }
}

/// Windows adapter GUIDs look like `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`;
/// Linux interface names are short and plain. Both are validated before they
/// reach a shell or D-Bus.
pub fn valid_adapter_id(id: &str) -> bool {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return false;
    }
    if let Some(inner) = trimmed.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')) {
        return inner.len() == 36
            && inner
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-');
    }
    trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_cleanup_task_adapter_ids_are_validated_strictly() {
        assert!(valid_adapter_id("{12345678-1234-1234-1234-123456789ABC}"));
        assert!(valid_adapter_id("eth0"));
        assert!(valid_adapter_id("enp0s31f6"));
        assert!(!valid_adapter_id("{12345678-1234-1234-1234-123456789AB}"));
        assert!(!valid_adapter_id("eth0; rm -rf /"));
        assert!(!valid_adapter_id("eth 0"));
        assert!(!valid_adapter_id(""));
    }

    #[test]
    fn lan_cleanup_task_state_round_trip() {
        let state = UplinkState {
            sharing: Some(SharingRecord {
                private_index: 2,
                private_name: "eth0".into(),
                private_id: "eth0".into(),
                public_index: 3,
                public_name: "wlan0".into(),
                public_id: "wlan0".into(),
                since: 7,
            }),
            last_error: None,
        };
        assert_eq!(UplinkState::parse(&state.encode().unwrap()).unwrap(), state);
        assert_eq!(UplinkState::parse("").unwrap(), UplinkState::default());
    }
}
