//! Administrative policy and exact, independently signed peer operations (V1).
use serde::{Deserialize, Serialize};
use crate::screen::core::input_model::{InputEvent, MAX_INPUT_EVENTS, MAX_TEXT_BYTES};

pub(crate) const VERSION: u32 = 1;
pub(crate) const MAX_CONTROL: usize = 64 * 1024;
pub(crate) const MAX_POLICY: usize = 1024 * 1024;
pub(crate) const MAX_PIXELS: usize = 4096 * 2304 * 4;
pub(crate) const BROKER_SWITCH: &str = "--login-broker-service";
pub(crate) const HELPER_SWITCH: &str = "--login-helper";
pub(crate) const PROBE_SWITCH: &str = "--login-session-probe";
pub(crate) const GATEWAY_SWITCH: &str = "--login-gateway-service";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopKind { BeforeLogin, Locked, Uac, UserSession, X11Greeter, WaylandGreeter, Unavailable }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionBinding {
    pub session_id: String,
    pub user_id: Option<String>,
    pub session_generation: String,
    pub desktop_generation: String,
    pub desktop: DesktopKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRights {
    pub screen_view: bool,
    pub screen_control: bool,
    pub login: bool,
    pub before_login: bool,
    pub locked: bool,
    pub uac: bool,
    pub sas: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeerGrant {
    pub peer_id: String,
    pub public_key: String,
    pub target_users: Vec<String>,
    pub not_before: u64,
    pub expires_at: u64,
    pub rights: LoginRights,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    pub format: u32,
    pub revision: u64,
    pub machine_id: String,
    pub machine_public_key: String,
    /// Windows service SID or Linux numeric UID. Never an interactive user's SID.
    pub gateway_identity: String,
    pub grants: Vec<PeerGrant>,
}

impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.format != VERSION || self.revision == 0 || self.machine_id.is_empty()
            || self.machine_id.len() > 256 || self.gateway_identity.is_empty()
            || self.machine_public_key.parse::<iroh::PublicKey>().is_err() || self.grants.len() > 256 {
            return Err("invalid protected login policy");
        }
        let mut peers = std::collections::HashSet::new();
        for grant in &self.grants {
            if grant.peer_id.is_empty() || grant.peer_id.len() > 256
                || !peers.insert(&grant.peer_id) || grant.public_key.parse::<iroh::PublicKey>().is_err()
                || grant.target_users.is_empty() || grant.target_users.len() > 64
                || grant.target_users.iter().any(|id| id.is_empty() || id.len() > 256)
                || grant.not_before >= grant.expires_at {
                return Err("invalid protected login grant");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    Start,
    Capture { frame_id: u64 },
    Input { events: Vec<InputEvent> },
    ReleaseInputs,
    SecureAttention,
    Stop,
}

impl Operation {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if let Self::Input { events } = self {
            if events.is_empty() || events.len() > MAX_INPUT_EVENTS { return Err("invalid input batch size"); }
            for event in events {
                match event {
                    InputEvent::Text { text } if text.len() > MAX_TEXT_BYTES || text.contains('\0') => return Err("invalid input text"),
                    InputEvent::Key { hid_usage, .. } if !crate::screen::core::input_model::valid_key_usage(*hid_usage) => return Err("invalid input key"),
                    InputEvent::Action { .. } => return Err("Android actions are unavailable at login"),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Challenge {
    pub format: u32,
    pub machine_id: String,
    pub machine_public_key: String,
    pub boot_generation: String,
    pub binding: SessionBinding,
    pub policy_revision: u64,
    pub nonce: String,
    pub sequence: u64,
    pub expires_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedRequest {
    pub challenge: Challenge,
    pub peer_id: String,
    pub target_user: String,
    pub operation: Operation,
    pub signature: String,
}

impl SignedRequest {
    /// Fixed field order and domain prefix; this exact byte string is signed by the peer.
    pub(crate) fn signing_bytes(&self) -> std::io::Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Body<'a> { challenge: &'a Challenge, peer_id: &'a str, target_user: &'a str, operation: &'a Operation }
        let mut bytes = b"smart-explorer/login-operation/v1\0".to_vec();
        serde_json::to_writer(&mut bytes, &Body { challenge: &self.challenge, peer_id: &self.peer_id,
            target_user: &self.target_user, operation: &self.operation }).map_err(std::io::Error::other)?;
        if bytes.len() > MAX_CONTROL { return Err(std::io::Error::other("signed login operation exceeds limit")); }
        Ok(bytes)
    }

    pub(crate) fn sign(&mut self, key: &iroh::SecretKey) -> std::io::Result<()> {
        self.signature = crate::share::iroh_signature(key, &self.signing_bytes()?);
        Ok(())
    }
}
