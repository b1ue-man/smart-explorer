//! Serializable snapshot of the local-network state for the GUI and CLI.
//! Every facility reports availability explicitly; nothing is assumed.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LanFacility {
    #[default]
    Disabled,
    Starting,
    Available,
    Unavailable(String),
}

impl LanFacility {
    pub fn label(&self) -> String {
        match self {
            LanFacility::Disabled => "aus".into(),
            LanFacility::Starting => "startet".into(),
            LanFacility::Available => "aktiv".into(),
            LanFacility::Unavailable(reason) => format!("nicht verfuegbar: {reason}"),
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, LanFacility::Available)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanPeerView {
    pub contact_id: String,
    pub display_name: String,
    pub candidates: Vec<String>,
    pub uplink: Option<bool>,
    pub seen_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkView {
    pub name: String,
    pub index: u32,
    pub class: String,
    pub addrs: Vec<String>,
    pub has_gateway: bool,
    pub dhcp_lease: Option<bool>,
    /// A paired peer was seen on this link.
    pub peer_present: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UplinkSharingState {
    #[default]
    Idle,
    Starting,
    Sharing,
    Stopping,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UplinkView {
    pub enabled: bool,
    pub setup_done: bool,
    pub facility: LanFacility,
    pub state: UplinkSharingState,
    /// Why the policy is idle / what it is doing, for the user.
    pub reason: String,
    pub private_if: Option<String>,
    pub public_if: Option<String>,
    pub since: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanStatus {
    pub presence: LanFacility,
    /// The announced instance (hashed id) when presence runs.
    pub announced_id: Option<String>,
    pub peers: Vec<LanPeerView>,
    pub links: Vec<LinkView>,
    pub links_error: Option<String>,
    pub unknown_devices: usize,
    pub uplink: UplinkView,
}
