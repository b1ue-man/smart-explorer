//! `login_target_v1` (E22, plan 2026-11-08): a host sends the public target
//! of its installed machine login gateway to a device that may steer its
//! screen, over the pinned, TLS-authenticated Share session. The pairing
//! replaces the explicit machine pin acknowledgement of an imported
//! `gateway-target.json`; that import stays available.
//!
//! The host decides with [`host_message`]: an offer only for a device with a
//! current „Bildschirm steuern: Erlauben“ that the gateway's self-set grants
//! name, otherwise a withdrawal. The viewer accepts with [`accept`]: only
//! from the pinned session of the device the message names, and never a
//! target whose key is the device's own Share key. Every message stays
//! within `MAX_NETWORK` bytes.
use crate::login::core::gateway_types::{GatewayTarget, MAX_NETWORK};
use serde::{Deserialize, Serialize};
use std::io;

/// The capability name a host reports and a viewer asks for.
pub const LOGIN_TARGET_FEATURE: &str = "login_target_v1";
/// Format of [`LoginTargetMessage`].
pub const LOGIN_TARGET_FORMAT: u32 = 1;
/// Longest device ID or target account a message carries.
const MAX_NAME: usize = 256;

/// Why a host withdraws its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawReason {
    /// The device may not steer this screen (now).
    NoSteering,
    /// No machine login gateway runs on this host.
    NotInstalled,
    /// The gateway does not (yet) hold a grant for the device.
    NotGranted,
    /// A reason of a later version.
    #[serde(other)]
    Unknown,
}

impl WithdrawReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::NoSteering => "no_steering",
            Self::NotInstalled => "not_installed",
            Self::NotGranted => "not_granted",
            Self::Unknown => "unknown",
        }
    }
}

/// One host answer of `login_target_v1`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LoginTargetMessage {
    /// The installed gateway of the host `device_id`; `target_user` is the
    /// account (Linux UID, Windows SID) whose grant names the receiver.
    Offer {
        format: u32,
        device_id: String,
        target_user: String,
        target: GatewayTarget,
    },
    /// The host `device_id` offers no target to the receiver.
    Withdrawn {
        format: u32,
        device_id: String,
        reason: WithdrawReason,
    },
}

impl LoginTargetMessage {
    pub fn device_id(&self) -> &str {
        match self {
            Self::Offer { device_id, .. } | Self::Withdrawn { device_id, .. } => device_id,
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        let (format, device_id) = match self {
            Self::Offer {
                format, device_id, ..
            }
            | Self::Withdrawn {
                format, device_id, ..
            } => (*format, device_id),
        };
        if format != LOGIN_TARGET_FORMAT || !valid_name(device_id) {
            return Err(invalid("ungültige Anmeldeziel-Nachricht"));
        }
        if let Self::Offer {
            target_user,
            target,
            ..
        } = self
        {
            if !valid_name(target_user) {
                return Err(invalid("ungültiges Zielkonto im Anmeldeziel"));
            }
            target.validate()?;
        }
        Ok(())
    }

    /// The bytes on the wire; never more than `MAX_NETWORK`.
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(invalid)?;
        if bytes.len() > MAX_NETWORK {
            return Err(invalid("Anmeldeziel überschreitet 32 KiB"));
        }
        Ok(bytes)
    }

    /// Reads a message; the size is checked before parsing.
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_NETWORK {
            return Err(invalid("Anmeldeziel überschreitet 32 KiB"));
        }
        let message: Self = serde_json::from_slice(bytes).map_err(invalid)?;
        message.validate()?;
        Ok(message)
    }
}

/// What the host knows when a device asks for its target.
pub struct HostFacts<'a> {
    /// This host's own device ID.
    pub host_device_id: &'a str,
    /// The authenticated asking device.
    pub device_id: &'a str,
    /// `access_steering::steering_device` for the asking device, now.
    pub steering: bool,
    /// The broker's view (`login::self_setup::broker_view`); `None` when no
    /// broker answers.
    pub account: Option<&'a str>,
    pub own_grants: &'a [String],
    pub target: Option<&'a GatewayTarget>,
}

/// The host's answer: an offer only with a current steering right, a
/// running gateway and a self-set grant for exactly this device.
pub fn host_message(facts: &HostFacts<'_>) -> LoginTargetMessage {
    let withdrawn = |reason| LoginTargetMessage::Withdrawn {
        format: LOGIN_TARGET_FORMAT,
        device_id: facts.host_device_id.to_string(),
        reason,
    };
    if !facts.steering {
        return withdrawn(WithdrawReason::NoSteering);
    }
    let (Some(account), Some(target)) = (facts.account, facts.target) else {
        return withdrawn(WithdrawReason::NotInstalled);
    };
    if !valid_name(account) || target.validate().is_err() {
        return withdrawn(WithdrawReason::NotInstalled);
    }
    if !facts.own_grants.iter().any(|peer| peer == facts.device_id) {
        return withdrawn(WithdrawReason::NotGranted);
    }
    LoginTargetMessage::Offer {
        format: LOGIN_TARGET_FORMAT,
        device_id: facts.host_device_id.to_string(),
        target_user: account.to_string(),
        target: target.clone(),
    }
}

/// The authenticated Share session a message arrived on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPeer {
    /// The device the session authenticated (hello and grant).
    pub device_id: String,
    /// Its Share public key.
    pub public_key: String,
    /// Its TLS-authenticated Iroh node ID.
    pub node_id: String,
    /// Its name as this device shows it.
    pub device_name: String,
    /// Contact ID (Direct) or room profile ID.
    pub relation_id: String,
    /// The session is bound to the pinned identity of the contact.
    pub pinned: bool,
}

/// Why a viewer drops a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not from the pinned, authenticated session of a contact.
    NotPinned,
    /// The message names another device than the session.
    ForeignDevice,
    /// The target key is the device's own Share key.
    DeviceKey,
    /// Format, size or contents are invalid.
    Invalid,
}

impl Refusal {
    pub fn key(self) -> &'static str {
        match self {
            Self::NotPinned => "login_target_not_pinned",
            Self::ForeignDevice => "login_target_foreign_device",
            Self::DeviceKey => "login_target_device_key",
            Self::Invalid => "login_target_invalid",
        }
    }
}

/// What a viewer does with an accepted message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accepted {
    Store {
        target_user: String,
        target: GatewayTarget,
    },
    Remove(WithdrawReason),
}

/// The viewer's check before any message takes effect.
pub fn accept(peer: &SessionPeer, message: &LoginTargetMessage) -> Result<Accepted, Refusal> {
    if !peer.pinned || !valid_name(&peer.device_id) || peer.public_key.is_empty() {
        return Err(Refusal::NotPinned);
    }
    message.validate().map_err(|_| Refusal::Invalid)?;
    if message.device_id() != peer.device_id {
        return Err(Refusal::ForeignDevice);
    }
    match message {
        LoginTargetMessage::Offer {
            target_user,
            target,
            ..
        } => {
            if target.public_key == peer.public_key || target.public_key == peer.node_id {
                return Err(Refusal::DeviceKey);
            }
            Ok(Accepted::Store {
                target_user: target_user.clone(),
                target: target.clone(),
            })
        }
        LoginTargetMessage::Withdrawn { reason, .. } => Ok(Accepted::Remove(*reason)),
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
}

fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
