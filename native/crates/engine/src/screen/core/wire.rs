//! Framing of the screen control stream (strand S4, decision C3). A frame is
//! the Exec frame head of 5 bytes, tag `u8` and length `u32` big-endian,
//! written by `exec_protocol::write_payload`; the reader checks the tag's
//! individual byte budget before allocating its payload. The
//! payload of a tag is JSON or a fixed big-endian layout. Every length that
//! arrives from the network is checked against a fixed limit before memory
//! for it is requested.
use std::io;

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

use crate::share::exec_protocol::write_payload;

/// ALPN of the screen protocol. The endpoint keeps the FS, Exec and LAN-link
/// ALPNs; a peer without this one ends the TLS handshake with alert 120,
/// which the viewer sees as transport code `0x178` (NR N2).
pub const SCREEN_ALPN: &[u8] = b"smart-explorer/share-screen/1";
/// Version in both hellos. The viewer answers with the version the host
/// announced in `SERVER_HELLO` when it supports it.
pub const SCREEN_PROTOCOL_VERSION: u16 = 1;

// Handshake: the host announces itself, the viewer proves its relation, the
// host answers (optionally `APPROVAL_PENDING` first).
pub const SERVER_HELLO: u8 = 1;
pub const CLIENT_HELLO: u8 = 2;
pub const HELLO_OK: u8 = 3;
pub const HELLO_ERROR: u8 = 4;
pub const APPROVAL_PENDING: u8 = 5;
// Viewer to host.
pub const SET_MODE: u8 = 16;
pub const INPUT: u8 = 17;
pub const FRAME_ACK: u8 = 18;
pub const KEYFRAME_REQUEST: u8 = 19;
pub const PING: u8 = 20;
pub const LIST_WINDOWS: u8 = 21;
pub const SNAPSHOT: u8 = 22;
pub const CONTROL_REQUEST: u8 = 23;
pub const PREVIEW_ACK: u8 = 24;
pub const VISIBILITY: u8 = 25;
pub const ICON_REQUEST: u8 = 26;
pub const PRESENTED: u8 = 27;
pub const LIST_FAMILIES: u8 = 28;
pub const FAMILY_SNAPSHOT_REQUEST: u8 = 29;
pub const FAMILY_OPERATION: u8 = 30;
pub const FAMILY_INPUT: u8 = 31;
// Host to viewer. Tag 35 stays free: still images travel as a picture
// stream of the kind `snapshot`.
pub const STATE: u8 = 32;
pub const PONG: u8 = 33;
pub const WINDOWS: u8 = 34;
pub const STATS: u8 = 36;
pub const CONTROL_STATE: u8 = 37;
pub const ERROR: u8 = 38;
pub const VISIBILITY_ACK: u8 = 39;
pub const ICON_REPLY: u8 = 40;
pub const FAMILIES: u8 = 41;
pub const FAMILY_SNAPSHOT: u8 = 42;
pub const FAMILY_OPERATION_RESULT: u8 = 43;
pub const FAMILY_INPUT_RESULT: u8 = 44;
// Device feed (0.5.189, design 5.2): the viewer subscribes once on a
// metadata channel; the host then sends the full state on every change.
pub const SUBSCRIBE_FEED: u8 = 45;
pub const FEED_UPDATE: u8 = 46;
/// Version of the device feed a host announces in `SERVER_HELLO`.
pub const FAMILY_FEED_VERSION: u16 = 1;

/// Control frames except `WINDOWS` and the separately bounded icon frames;
/// the 64 KiB of `MAX_HANDSHAKE_BYTES`
/// in the Exec protocol.
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;
/// `WINDOWS`: the most every viewer of protocol version 1 reads; the
/// list has no entry count (R15c) and is bounded by these bytes alone
/// (`WindowList::bounded`).
pub const MAX_WINDOWS_BYTES: usize = 1024 * 1024;

/// Whether this build speaks the protocol version a host announced.
pub fn supports_version(version: u16) -> bool {
    version == SCREEN_PROTOCOL_VERSION
}

/// The byte limit of the payload of `tag`.
pub fn tag_limit(tag: u8) -> usize {
    match tag {
        WINDOWS | FAMILIES | FAMILY_SNAPSHOT | FAMILY_OPERATION_RESULT | FEED_UPDATE => {
            MAX_WINDOWS_BYTES
        }
        ICON_REQUEST => super::app_icon::MAX_ICON_REQUEST_BYTES,
        ICON_REPLY => super::app_icon::MAX_ICON_REPLY_BYTES,
        PRESENTED => 16,
        _ => MAX_CONTROL_BYTES,
    }
}

/// Writes one frame; a payload above the limit of its tag is refused before
/// anything is written.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> io::Result<()> {
    if payload.len() > tag_limit(tag) {
        return Err(invalid("screen frame exceeds the limit of its tag"));
    }
    write_payload(writer, tag, payload).await
}

/// Reads one frame. `limit` is the largest payload the reading side accepts
/// at all (`MAX_CONTROL_BYTES` on the host, `MAX_WINDOWS_BYTES` on the
/// viewer). Both the caller's ceiling and the individual tag ceiling are
/// checked from the fixed-size header before payload allocation.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header).await?;
    let tag = header[0];
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if length > limit.min(tag_limit(tag)) {
        return Err(invalid("screen frame exceeds the limit of its tag"));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    Ok((tag, payload))
}

pub fn to_json<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn from_json<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub async fn send_json<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    tag: u8,
    value: &T,
) -> io::Result<()> {
    let payload = to_json(value)?;
    write_frame(writer, tag, &payload).await
}

/// Reads one JSON frame of `expected_tag` (handshake order).
pub async fn recv_json<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
    expected_tag: u8,
) -> io::Result<T> {
    let (tag, payload) = read_frame(reader, MAX_CONTROL_BYTES).await?;
    if tag != expected_tag {
        return Err(invalid("unexpected screen frame"));
    }
    from_json(&payload)
}

/// Reads the fixed big-endian fields of one binary payload. Every read
/// checks the remaining length, so a length field never causes a read past
/// the payload or an allocation of its own.
pub struct BeReader<'a> {
    data: &'a [u8],
}

impl<'a> BeReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
        if self.data.len() < len {
            return Err(invalid("screen binary payload is too short"));
        }
        let (head, rest) = self.data.split_at(len);
        self.data = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        self.array().map(u16::from_be_bytes)
    }

    pub fn i16(&mut self) -> io::Result<i16> {
        self.array().map(i16::from_be_bytes)
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        self.array().map(u32::from_be_bytes)
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        self.array().map(u64::from_be_bytes)
    }

    pub fn remaining(&self) -> usize {
        self.data.len()
    }

    /// Refuses bytes after the last field.
    pub fn finish(&self) -> io::Result<()> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(invalid("screen binary payload has trailing bytes"))
        }
    }
}

pub fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
