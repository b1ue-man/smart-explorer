//! Framing of the screen control stream (strand S4, decision C3). A frame is
//! the Exec frame head of 5 bytes, tag `u8` and length `u32` big-endian,
//! written and read by `exec_protocol::{write_payload, read_payload}`. The
//! payload of a tag is JSON or a fixed big-endian layout. Every length that
//! arrives from the network is checked against a fixed limit before memory
//! for it is requested.
use std::io;

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::share::exec_protocol::{read_payload, write_payload};

/// ALPN of the screen protocol. The endpoint keeps the FS, Exec and LAN-link
/// ALPNs; a peer without this one ends the TLS handshake with alert 120,
/// which the viewer sees as transport code `0x178` (NR N2).
pub(crate) const SCREEN_ALPN: &[u8] = b"smart-explorer/share-screen/1";
/// Version in both hellos. The viewer answers with the version the host
/// announced in `SERVER_HELLO` when it supports it.
pub(crate) const SCREEN_PROTOCOL_VERSION: u16 = 1;

// Handshake: the host announces itself, the viewer proves its relation, the
// host answers (optionally `APPROVAL_PENDING` first).
pub(crate) const SERVER_HELLO: u8 = 1;
pub(crate) const CLIENT_HELLO: u8 = 2;
pub(crate) const HELLO_OK: u8 = 3;
pub(crate) const HELLO_ERROR: u8 = 4;
pub(crate) const APPROVAL_PENDING: u8 = 5;
// Viewer to host.
pub(crate) const SET_MODE: u8 = 16;
pub(crate) const INPUT: u8 = 17;
pub(crate) const FRAME_ACK: u8 = 18;
pub(crate) const KEYFRAME_REQUEST: u8 = 19;
pub(crate) const PING: u8 = 20;
pub(crate) const LIST_WINDOWS: u8 = 21;
pub(crate) const SNAPSHOT: u8 = 22;
pub(crate) const CONTROL_REQUEST: u8 = 23;
pub(crate) const PREVIEW_ACK: u8 = 24;
// Host to viewer. Tag 35 stays free: still images travel as a picture
// stream of the kind `snapshot`.
pub(crate) const STATE: u8 = 32;
pub(crate) const PONG: u8 = 33;
pub(crate) const WINDOWS: u8 = 34;
pub(crate) const STATS: u8 = 36;
pub(crate) const CONTROL_STATE: u8 = 37;
pub(crate) const ERROR: u8 = 38;

/// Every control frame except `WINDOWS`; the 64 KiB of `MAX_HANDSHAKE_BYTES`
/// in the Exec protocol.
pub(crate) const MAX_CONTROL_BYTES: usize = 64 * 1024;
/// `WINDOWS`: up to 4096 entries of up to 256 bytes each.
pub(crate) const MAX_WINDOWS_BYTES: usize = 1024 * 1024;

/// Whether this build speaks the protocol version a host announced.
pub(crate) fn supports_version(version: u16) -> bool {
    version == SCREEN_PROTOCOL_VERSION
}

/// The byte limit of the payload of `tag`.
pub(crate) fn tag_limit(tag: u8) -> usize {
    if tag == WINDOWS {
        MAX_WINDOWS_BYTES
    } else {
        MAX_CONTROL_BYTES
    }
}

/// Writes one frame; a payload above the limit of its tag is refused before
/// anything is written.
pub(crate) async fn write_frame<W: AsyncWrite + Unpin>(
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
/// viewer); a longer head is refused before the payload is allocated, and a
/// payload above the limit of its own tag afterwards.
pub(crate) async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<(u8, Vec<u8>)> {
    let (tag, payload) = read_payload(reader, limit).await.map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            invalid("screen frame exceeds its byte limit")
        } else {
            error
        }
    })?;
    if payload.len() > tag_limit(tag) {
        return Err(invalid("screen frame exceeds the limit of its tag"));
    }
    Ok((tag, payload))
}

pub(crate) fn to_json<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn from_json<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) async fn send_json<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    tag: u8,
    value: &T,
) -> io::Result<()> {
    let payload = to_json(value)?;
    write_frame(writer, tag, &payload).await
}

/// Reads one JSON frame of `expected_tag` (handshake order).
pub(crate) async fn recv_json<R: AsyncRead + Unpin, T: DeserializeOwned>(
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
pub(crate) struct BeReader<'a> {
    data: &'a [u8],
}

impl<'a> BeReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
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

    pub(crate) fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> io::Result<u16> {
        self.array().map(u16::from_be_bytes)
    }

    pub(crate) fn i16(&mut self) -> io::Result<i16> {
        self.array().map(i16::from_be_bytes)
    }

    pub(crate) fn u32(&mut self) -> io::Result<u32> {
        self.array().map(u32::from_be_bytes)
    }

    pub(crate) fn u64(&mut self) -> io::Result<u64> {
        self.array().map(u64::from_be_bytes)
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len()
    }

    /// Refuses bytes after the last field.
    pub(crate) fn finish(&self) -> io::Result<()> {
        if self.data.is_empty() {
            Ok(())
        } else {
            Err(invalid("screen binary payload has trailing bytes"))
        }
    }
}

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
