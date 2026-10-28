//! One typed, request-bound device command per authenticated connection.
use super::relation_proof::{RelationClientHello, RelationServerHello};
use crate::apps::{AppId, CatalogSnapshot, LaunchReceipt, LaunchRequest};
use crate::screen::core::app_icon::{IconRef, IconReply};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const APPS_ALPN: &[u8] = b"smart-explorer/share-apps/1";
pub(crate) const APPS_VERSION: u16 = 1;
/// Leave room for the authenticated daemon envelope within its existing 2 MiB
/// line. Oversized inventories are refused whole, never silently truncated.
pub(crate) const MAX_PACKET: usize = 2 * 1024 * 1024 - 16 * 1024;
const MAX_HELLO: usize = 16 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppsCommand {
    Catalog,
    Icon {
        app_id: AppId,
        icon: IconRef,
        catalog_revision: String,
    },
    Launch {
        request: LaunchRequest,
    },
    Receipt {
        request: LaunchRequest,
    },
    RdpProbe,
    /// Design 9.2 (0.5.189). Hosts up to 0.5.188 refuse these variants as
    /// unknown; the viewer then reports „Gegenstelle älter als 0.5.189“.
    RdpProbeV2,
    /// T4: run the host's RDP self-setup, then probe again.
    RdpPrepare,
    /// se-managed service credentials (GNOME desktop sharing), new per call.
    RdpCredential {
        service_id: String,
    },
    /// Locally listening services that steering exposes (CLI/MCP without feed).
    LocalServices,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppsResult {
    Catalog {
        catalog: CatalogSnapshot,
    },
    Icon {
        reply: Option<IconReply>,
    },
    Launch {
        receipt: LaunchReceipt,
    },
    Receipt {
        receipt: Option<LaunchReceipt>,
    },
    RdpProbe {
        host: crate::rdp::HostProbe,
    },
    RdpProbeV2 {
        host: crate::rdp::HostProbe,
        setup: crate::rdp::core::SetupState,
    },
    RdpPrepare {
        setup: crate::rdp::core::SetupState,
        host: crate::rdp::HostProbe,
    },
    RdpCredential {
        username: String,
        password: ManagedSecret,
    },
    LocalServices {
        services: Vec<super::loopback_services::LoopbackService>,
        unavailable: Option<String>,
    },
}

/// A credential se generated for one RDP start. Serialized as the plain
/// string inside the authenticated reply; its Debug form never shows it, so
/// a logged `AppsResult` cannot leak the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ManagedSecret(String);
impl ManagedSecret {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
    // Consumer: the viewer handing the credential to its client (WP2).
    #[allow(dead_code)]
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for ManagedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedSecret(<redacted>)")
    }
}
impl Drop for ManagedSecret {
    fn drop(&mut self) {
        // Overwrite the bytes before the allocation is released.
        // SAFETY: zero bytes keep the String valid UTF-8.
        unsafe { self.0.as_bytes_mut() }.fill(0);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppsServerHello {
    pub version: u16,
    pub challenge: String,
    pub device_id: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppsClientHello {
    pub version: u16,
    pub relation_kind: String,
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub nonce: String,
    pub proof: String,
    pub id: String,
    pub command: AppsCommand,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppsReply {
    Pending,
    Refused {
        detail: String,
    },
    Completed {
        id: String,
        request_hash: String,
        result: AppsResult,
    },
}
impl AppsServerHello {
    pub(crate) fn relation(&self, client: &AppsClientHello) -> io::Result<RelationServerHello> {
        let bound =
            serde_json::to_string(&(&client.id, &client.command)).map_err(io::Error::other)?;
        Ok(RelationServerHello {
            transcript_prefix: format!("smart-explorer/share-apps/v1|{}:{bound}|", bound.len()),
            protocol_version: APPS_VERSION,
            capability: "device_apps".into(),
            challenge: self.challenge.clone(),
            server_device_id: self.device_id.clone(),
            server_public_key: self.public_key.clone(),
            server_fingerprint: self.fingerprint.clone(),
            server_node_id: self.node_id.clone(),
        })
    }
}
impl AppsClientHello {
    pub(crate) fn relation(&self) -> RelationClientHello {
        RelationClientHello {
            protocol_version: self.version,
            capability: "device_apps".into(),
            relation_kind: self.relation_kind.clone(),
            relation_id: self.relation_id.clone(),
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            public_key: self.public_key.clone(),
            fingerprint: self.fingerprint.clone(),
            node_id: self.node_id.clone(),
            client_nonce: self.nonce.clone(),
            proof: self.proof.clone(),
        }
    }
    pub(crate) fn request_hash(&self) -> io::Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&self.id, &self.command)).map_err(io::Error::other)?
            )
        ))
    }
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.id.is_empty()
            || self.id.len() > 128
            || self.id.chars().any(char::is_control)
            || self.nonce.len() > 256
            || self.proof.len() > 256
            || self.relation_id.len() > 1024
            || self.device_name.len() > 4096
            || self.device_id.len() > 1024
            || self.public_key.len() > 1024
            || self.fingerprint.len() > 1024
            || self.node_id.len() > 1024
        {
            return Err(io::Error::other("invalid device-command identity envelope"));
        }
        Ok(())
    }
}
pub(crate) async fn send<W: AsyncWrite + Unpin, T: Serialize>(
    out: &mut W,
    value: &T,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_PACKET {
        return Err(io::Error::other("complete device-command result exceeds bounded IPC-compatible frame; no partial inventory returned"));
    }
    out.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    out.write_all(&bytes).await?;
    out.flush().await
}
pub(crate) async fn receive<R: AsyncRead + Unpin, T: DeserializeOwned>(
    input: &mut R,
) -> io::Result<T> {
    receive_bounded(input, MAX_PACKET).await
}
pub(crate) async fn receive_hello<R: AsyncRead + Unpin, T: DeserializeOwned>(
    input: &mut R,
) -> io::Result<T> {
    receive_bounded(input, MAX_HELLO).await
}
async fn receive_bounded<R: AsyncRead + Unpin, T: DeserializeOwned>(
    input: &mut R,
    budget: usize,
) -> io::Result<T> {
    let length = input.read_u32().await? as usize;
    if length == 0 || length > budget {
        return Err(io::Error::other("invalid device-command packet length"));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
