//! Only paired link status. These messages cannot name filesystem/Exec actions.
use serde::{Deserialize, Serialize};

use super::direct_protocol::DirectPeerIdentity;
use super::lan_link_facts::OwnUplink;

pub(crate) const LAN_LINK_ALPN: &[u8] = b"smart-explorer/paired-link/1";
pub(crate) const MAX_LINK_FRAME: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub(super) enum LinkFrame {
    Challenge { nonce: [u8; 32], identity: DirectPeerIdentity },
    Answer { echo: [u8; 32], nonce: [u8; 32], identity: DirectPeerIdentity, uplink: OwnUplink },
    Confirm { echo: [u8; 32], uplink: OwnUplink },
}

pub(super) fn check_echo(expected: &[u8; 32], received: &[u8; 32]) -> std::io::Result<()> {
    if expected.iter().any(|byte| *byte != 0) && expected == received { Ok(()) }
    else { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "LAN-Link-Challenge passt nicht")) }
}

pub(super) fn encode(frame: &LinkFrame) -> std::io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(frame).map_err(super::core::eio)?;
    if bytes.is_empty() || bytes.len() > MAX_LINK_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "LAN-Link-Frame zu gross"));
    }
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> std::io::Result<LinkFrame> {
    if bytes.is_empty() || bytes.len() > MAX_LINK_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "LAN-Link-Frame zu gross"));
    }
    let frame: LinkFrame = serde_json::from_slice(bytes).map_err(super::core::eio)?;
    let identity = match &frame {
        LinkFrame::Challenge { identity, .. } | LinkFrame::Answer { identity, .. } => Some(identity),
        LinkFrame::Confirm { .. } => None,
    };
    if let Some(identity) = identity {
        if !identity.device_name.is_empty() { return Err(super::core::eio("LAN-Link sendet keinen Rechnernamen")); }
        identity.validate().map_err(super::core::eio)?;
    }
    Ok(frame)
}
