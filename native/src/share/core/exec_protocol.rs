use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::exec_types::{
    ExecAuthorization, ExecId, ExecProviderStatus, ExecStart, ExecTerminal, EXEC_CAPABILITY,
    EXEC_PROTOCOL_VERSION, MAX_EXEC_DATA_BYTES, MAX_EXEC_START_BYTES,
};

pub(crate) const EXEC_ALPN: &[u8] = b"smart-explorer/share-exec/2";
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const HEADER_BYTES: usize = 5;

const SERVER_HELLO: u8 = 1;
const CLIENT_HELLO: u8 = 2;
const HELLO_OK: u8 = 3;
const HELLO_ERROR: u8 = 4;
const START: u8 = 16;
const STDIN: u8 = 17;
const STDIN_EOF: u8 = 18;
const CANCEL: u8 = 19;
const PING: u8 = 20;
const RESULT_ACK: u8 = 21;
const STARTED: u8 = 32;
const STDOUT: u8 = 33;
const STDERR: u8 = 34;
const TERMINAL: u8 = 35;
const EXEC_ERROR: u8 = 36;
const PONG: u8 = 37;
const RESULT_ACKNOWLEDGED: u8 = 38;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecServerHello {
    pub protocol_version: u16,
    pub capability: String,
    pub challenge: String,
    pub server_device_id: String,
    pub server_public_key: String,
    pub server_fingerprint: String,
    pub server_node_id: String,
}

impl ExecServerHello {
    pub(crate) fn new(
        challenge: String,
        server_device_id: String,
        server_public_key: String,
        server_fingerprint: String,
        server_node_id: String,
    ) -> Self {
        Self {
            protocol_version: EXEC_PROTOCOL_VERSION,
            capability: EXEC_CAPABILITY.into(),
            challenge,
            server_device_id,
            server_public_key,
            server_fingerprint,
            server_node_id,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecClientHello {
    pub protocol_version: u16,
    pub capability: String,
    pub relation_kind: String,
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub client_nonce: String,
    pub proof: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecHelloOk {
    pub authorization: ExecAuthorization,
    pub provider: ExecProviderStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecWireError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClientFrame {
    Start { start: ExecStart, digest: String },
    Stdin { exec_id: ExecId, data: Vec<u8> },
    StdinEof { exec_id: ExecId },
    Cancel { exec_id: ExecId },
    Ping { exec_id: ExecId, sequence: u64 },
    ResultAck { exec_id: ExecId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerFrame {
    Started { exec_id: ExecId },
    Stdout { exec_id: ExecId, data: Vec<u8> },
    Stderr { exec_id: ExecId, data: Vec<u8> },
    Terminal(ExecTerminal),
    Error(ExecWireError),
    Pong { exec_id: ExecId, sequence: u64 },
    ResultAcknowledged { exec_id: ExecId },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ServerProtocolState {
    #[default]
    AwaitStart,
    Running,
    StdinClosed,
    Cancelling,
    Terminal,
}

impl ServerProtocolState {
    pub(crate) fn accept(&mut self, frame: &ClientFrame) -> io::Result<()> {
        match (*self, frame) {
            (Self::AwaitStart, ClientFrame::Start { start, digest }) => {
                start.validate()?;
                if &start.digest()? != digest {
                    return Err(invalid("exec start digest mismatch"));
                }
                *self = Self::Running;
            }
            (Self::Running, ClientFrame::Stdin { data, .. })
                if data.len() <= MAX_EXEC_DATA_BYTES => {}
            (Self::Running, ClientFrame::StdinEof { .. }) => *self = Self::StdinClosed,
            (Self::Running | Self::StdinClosed, ClientFrame::Cancel { .. }) => {
                *self = Self::Cancelling
            }
            (Self::Running | Self::StdinClosed | Self::Cancelling, ClientFrame::Ping { .. }) => {}
            // Frames written before the peer observed Terminal can still be
            // queued ahead of ResultAck on the ordered client stream. They no
            // longer affect the finished job, but must be drained so the
            // acknowledgement can close the terminal-result handshake.
            (Self::Terminal, ClientFrame::Stdin { data, .. })
                if data.len() <= MAX_EXEC_DATA_BYTES => {}
            (
                Self::Terminal,
                ClientFrame::StdinEof { .. }
                | ClientFrame::Cancel { .. }
                | ClientFrame::Ping { .. }
                | ClientFrame::ResultAck { .. },
            ) => {}
            (_, ClientFrame::Stdin { data, .. }) if data.len() > MAX_EXEC_DATA_BYTES => {
                return Err(invalid("exec stdin chunk exceeds 64 KiB"));
            }
            _ => return Err(invalid("exec client frame is out of order")),
        }
        Ok(())
    }

    pub(crate) fn terminal(&mut self) {
        *self = Self::Terminal;
    }
}

pub(crate) async fn send_server_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ExecServerHello,
) -> io::Result<()> {
    send_json(writer, SERVER_HELLO, hello, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn recv_server_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ExecServerHello> {
    recv_json(reader, SERVER_HELLO, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn send_client_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ExecClientHello,
) -> io::Result<()> {
    send_json(writer, CLIENT_HELLO, hello, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn recv_client_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ExecClientHello> {
    recv_json(reader, CLIENT_HELLO, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn send_hello_ok<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ExecHelloOk,
) -> io::Result<()> {
    send_json(writer, HELLO_OK, hello, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn send_hello_error<W: AsyncWrite + Unpin>(
    writer: &mut W,
    error: &ExecWireError,
) -> io::Result<()> {
    send_json(writer, HELLO_ERROR, error, MAX_HANDSHAKE_BYTES).await
}

pub(crate) async fn recv_hello_result<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<Result<ExecHelloOk, ExecWireError>> {
    let (tag, payload) = read_payload(reader, MAX_HANDSHAKE_BYTES).await?;
    match tag {
        HELLO_OK => decode(&payload).map(Ok),
        HELLO_ERROR => decode(&payload).map(Err),
        _ => Err(invalid("unexpected exec handshake response")),
    }
}

pub(crate) async fn send_client_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &ClientFrame,
) -> io::Result<()> {
    match frame {
        ClientFrame::Start { start, digest } => {
            start.validate()?;
            send_json(writer, START, &(start, digest), MAX_EXEC_START_BYTES).await
        }
        ClientFrame::Stdin { exec_id, data } => send_data(writer, STDIN, exec_id, data).await,
        ClientFrame::StdinEof { exec_id } => send_json(writer, STDIN_EOF, exec_id, 128).await,
        ClientFrame::Cancel { exec_id } => send_json(writer, CANCEL, exec_id, 128).await,
        ClientFrame::Ping { exec_id, sequence } => {
            send_json(writer, PING, &(exec_id, sequence), 256).await
        }
        ClientFrame::ResultAck { exec_id } => send_json(writer, RESULT_ACK, exec_id, 128).await,
    }
}

pub(crate) async fn recv_client_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ClientFrame> {
    let (tag, payload) =
        read_payload(reader, MAX_EXEC_START_BYTES.max(MAX_EXEC_DATA_BYTES + 32)).await?;
    match tag {
        START => {
            let (start, digest) = decode(&payload)?;
            Ok(ClientFrame::Start { start, digest })
        }
        STDIN => {
            let (exec_id, data) = decode_data(payload)?;
            Ok(ClientFrame::Stdin { exec_id, data })
        }
        STDIN_EOF => Ok(ClientFrame::StdinEof {
            exec_id: decode(&payload)?,
        }),
        CANCEL => Ok(ClientFrame::Cancel {
            exec_id: decode(&payload)?,
        }),
        PING => {
            let (exec_id, sequence) = decode(&payload)?;
            Ok(ClientFrame::Ping { exec_id, sequence })
        }
        RESULT_ACK => Ok(ClientFrame::ResultAck {
            exec_id: decode(&payload)?,
        }),
        _ => Err(invalid("unknown exec client frame")),
    }
}

pub(crate) async fn send_server_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &ServerFrame,
) -> io::Result<()> {
    match frame {
        ServerFrame::Started { exec_id } => send_json(writer, STARTED, exec_id, 128).await,
        ServerFrame::Stdout { exec_id, data } => send_data(writer, STDOUT, exec_id, data).await,
        ServerFrame::Stderr { exec_id, data } => send_data(writer, STDERR, exec_id, data).await,
        ServerFrame::Terminal(terminal) => {
            send_json(writer, TERMINAL, terminal, MAX_HANDSHAKE_BYTES).await
        }
        ServerFrame::Error(error) => {
            send_json(writer, EXEC_ERROR, error, MAX_HANDSHAKE_BYTES).await
        }
        ServerFrame::Pong { exec_id, sequence } => {
            send_json(writer, PONG, &(exec_id, sequence), 256).await
        }
        ServerFrame::ResultAcknowledged { exec_id } => {
            send_json(writer, RESULT_ACKNOWLEDGED, exec_id, 128).await
        }
    }
}

pub(crate) async fn recv_server_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ServerFrame> {
    let (tag, payload) =
        read_payload(reader, MAX_HANDSHAKE_BYTES.max(MAX_EXEC_DATA_BYTES + 32)).await?;
    match tag {
        STARTED => Ok(ServerFrame::Started {
            exec_id: decode(&payload)?,
        }),
        STDOUT => {
            let (exec_id, data) = decode_data(payload)?;
            Ok(ServerFrame::Stdout { exec_id, data })
        }
        STDERR => {
            let (exec_id, data) = decode_data(payload)?;
            Ok(ServerFrame::Stderr { exec_id, data })
        }
        TERMINAL => Ok(ServerFrame::Terminal(decode(&payload)?)),
        EXEC_ERROR => Ok(ServerFrame::Error(decode(&payload)?)),
        PONG => {
            let (exec_id, sequence) = decode(&payload)?;
            Ok(ServerFrame::Pong { exec_id, sequence })
        }
        RESULT_ACKNOWLEDGED => Ok(ServerFrame::ResultAcknowledged {
            exec_id: decode(&payload)?,
        }),
        _ => Err(invalid("unknown exec server frame")),
    }
}

async fn send_json<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    tag: u8,
    value: &T,
    limit: usize,
) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(super::core::eio)?;
    if payload.len() > limit {
        return Err(invalid("exec JSON frame exceeds its byte limit"));
    }
    write_payload(writer, tag, &payload).await
}

async fn recv_json<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
    expected_tag: u8,
    limit: usize,
) -> io::Result<T> {
    let (tag, payload) = read_payload(reader, limit).await?;
    if tag != expected_tag {
        return Err(invalid("unexpected exec frame"));
    }
    decode(&payload)
}

async fn send_data<W: AsyncWrite + Unpin>(
    writer: &mut W,
    tag: u8,
    exec_id: &ExecId,
    data: &[u8],
) -> io::Result<()> {
    if data.len() > MAX_EXEC_DATA_BYTES {
        return Err(invalid("exec data chunk exceeds 64 KiB"));
    }
    let mut payload = Vec::with_capacity(32 + data.len());
    payload.extend_from_slice(exec_id.as_str().as_bytes());
    payload.extend_from_slice(data);
    write_payload(writer, tag, &payload).await
}

fn decode_data(payload: Vec<u8>) -> io::Result<(ExecId, Vec<u8>)> {
    if payload.len() < 32 || payload.len() > 32 + MAX_EXEC_DATA_BYTES {
        return Err(invalid("invalid exec data frame length"));
    }
    let id = std::str::from_utf8(&payload[..32]).map_err(super::core::eio)?;
    Ok((ExecId::parse(id)?, payload[32..].to_vec()))
}

async fn write_payload<W: AsyncWrite + Unpin>(
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| invalid("exec frame length overflow"))?;
    writer.write_all(&[tag]).await?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

async fn read_payload<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; HEADER_BYTES];
    reader.read_exact(&mut header).await?;
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if len > limit {
        return Err(invalid("exec frame exceeds its byte limit"));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok((header[0], payload))
}

fn decode<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload).map_err(super::core::eio)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "exec_protocol_tests.rs"]
mod tests;
