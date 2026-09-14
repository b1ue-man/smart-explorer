//! User settings for the local-network features. Presence (finding paired
//! devices over mDNS) is on by default; sharing the internet uplink is an
//! explicit opt-in that also records whether the one-time platform setup ran.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanSettings {
    #[serde(default = "default_true")]
    pub presence_enabled: bool,
    #[serde(default)]
    pub uplink_sharing_enabled: bool,
    #[serde(default)]
    pub uplink_setup_done: bool,
    /// One-shot request from the GUI/CLI to stop an active sharing session;
    /// the daemon clears it after acting.
    #[serde(default)]
    pub uplink_stop_requested_at: Option<i64>,
}

fn default_true() -> bool {
    true
}

impl Default for LanSettings {
    fn default() -> Self {
        Self {
            presence_enabled: true,
            uplink_sharing_enabled: false,
            uplink_setup_done: false,
            uplink_stop_requested_at: None,
        }
    }
}

impl LanSettings {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(text).map_err(|error| format!("LAN-Einstellungen lesen: {error}"))
    }

    pub fn encode(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| format!("LAN-Einstellungen kodieren: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::LanSettings;

    #[test]
    fn defaults_and_round_trip() {
        let settings = LanSettings::parse("").unwrap();
        assert!(settings.presence_enabled);
        assert!(!settings.uplink_sharing_enabled);
        let partial = LanSettings::parse(r#"{"uplink_sharing_enabled":true}"#).unwrap();
        assert!(partial.presence_enabled);
        assert!(partial.uplink_sharing_enabled);
        let encoded = partial.encode().unwrap();
        assert_eq!(LanSettings::parse(&encoded).unwrap(), partial);
        assert!(LanSettings::parse("{nonsense").is_err());
    }
}
