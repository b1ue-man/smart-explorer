//! Length-delimited JSON only during setup; connected streams carry raw TCP
//! bytes. The relation proof also covers every destination/setup field.
use super::relation_proof::{RelationClientHello, RelationServerHello};
use super::tcp_forward::{TcpStart, TunnelId, TCP_PROTOCOL_VERSION};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_TCP_JSON: usize = 32 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpServerHello {
    pub(crate) version: u16,
    pub(crate) challenge: String,
    pub(crate) device_id: String,
    pub(crate) public_key: String,
    pub(crate) fingerprint: String,
    pub(crate) node_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpClientHello {
    pub(crate) version: u16,
    pub(crate) relation_kind: String,
    pub(crate) relation_id: String,
    pub(crate) device_id: String,
    pub(crate) device_name: String,
    pub(crate) public_key: String,
    pub(crate) fingerprint: String,
    pub(crate) node_id: String,
    pub(crate) nonce: String,
    pub(crate) proof: String,
    pub(crate) tunnel: TunnelId,
    pub(crate) request: TcpStart,
}
impl TcpServerHello {
    pub(crate) fn relation(&self, client: &TcpClientHello) -> io::Result<RelationServerHello> {
        let setup = serde_json::to_string(&(client.tunnel.clone(), &client.request))
            .map_err(io::Error::other)?;
        Ok(RelationServerHello {
            transcript_prefix: format!("smart-explorer/share-tcp/v1|{}:{setup}|", setup.len()),
            protocol_version: TCP_PROTOCOL_VERSION,
            capability: "tcp_forward".into(),
            challenge: self.challenge.clone(),
            server_device_id: self.device_id.clone(),
            server_public_key: self.public_key.clone(),
            server_fingerprint: self.fingerprint.clone(),
            server_node_id: self.node_id.clone(),
        })
    }
}
impl TcpClientHello {
    pub(crate) fn relation(&self) -> RelationClientHello {
        RelationClientHello {
            protocol_version: self.version,
            capability: "tcp_forward".into(),
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TcpHelloReply {
    Pending,
    Opened { target: std::net::SocketAddr },
    Refused { detail: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TcpDataOpen {
    pub(crate) tunnel: TunnelId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TcpDataReply {
    Connected,
    Refused { detail: String },
}

pub(crate) async fn send_json<W: AsyncWrite + Unpin, T: Serialize>(
    send: &mut W,
    value: &T,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_TCP_JSON {
        return Err(io::Error::other("TCP setup exceeds 32 KiB"));
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    send.write_all(&bytes).await?;
    send.flush().await
}
pub(crate) async fn recv_json<R: AsyncRead + Unpin, T: DeserializeOwned>(
    recv: &mut R,
) -> io::Result<T> {
    let length = recv.read_u32().await? as usize;
    if length == 0 || length > MAX_TCP_JSON {
        return Err(io::Error::other("invalid TCP setup length"));
    }
    let mut bytes = vec![0; length];
    recv.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
