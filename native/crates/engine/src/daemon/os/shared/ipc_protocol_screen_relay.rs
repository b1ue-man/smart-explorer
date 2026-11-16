//! The relay framing of `ScreenIpcRequest::Stream` (moved out of
//! `ipc_protocol_screen.rs` unchanged): one frame is tag `u8`, length `u32`
//! big-endian, payload; a picture record carries its picture group's head,
//! the optional length-prefixed v2 contract/family-revision JSON, the record
//! head and the payload.
use serde::{Deserialize, Serialize};

use super::RELAY_RECORD;
use crate::screen::core::wire::BeReader;
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, GOP_HEADER_BYTES, RECORD_HEADER_BYTES,
};

/// Same flattened v2 extension as the remote picture stream. Legacy v2 JSON
/// without a family revision retains exactly its existing meaning.
#[derive(Serialize, Deserialize)]
struct RelayGopExtension {
    #[serde(flatten)]
    contract: crate::screen::core::frame_budget::FrameContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family_layout_revision: Option<u64>,
}

/// The relay payload of one picture record: its picture group's head, the
/// record head and the payload.
pub fn encode_relay_record(header: &GopHeader, record: &FrameRecord) -> std::io::Result<Vec<u8>> {
    let len = u32::try_from(record.payload.len())
        .map_err(|_| std::io::Error::other("picture record is too large"))?;
    let head = header.encode()?;
    if record.payload.len() > header.record_limit() {
        return Err(std::io::Error::other(
            "picture record exceeds its frame contract",
        ));
    }
    let extension = if let Some(contract) = header.frame_contract {
        let extension = RelayGopExtension {
            contract,
            family_layout_revision: header.family_layout_revision,
        };
        let bytes = serde_json::to_vec(&extension).map_err(std::io::Error::other)?;
        if bytes.len() > 1024 {
            return Err(std::io::Error::other("screen relay contract exceeds 1 KiB"));
        }
        Some(bytes)
    } else {
        None
    };
    let overhead = GOP_HEADER_BYTES
        + RECORD_HEADER_BYTES
        + extension.as_ref().map_or(0, |bytes| bytes.len() + 2);
    let capacity = record
        .payload
        .len()
        .checked_add(overhead)
        .ok_or_else(|| std::io::Error::other("screen relay record length overflow"))?;
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&head);
    if let Some(bytes) = extension {
        body.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        body.extend_from_slice(&bytes);
    }
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(&record.frame_id.to_be_bytes());
    body.extend_from_slice(&record.capture_ts_us.to_be_bytes());
    body.push(record.flags);
    body.extend_from_slice(&record.payload);
    Ok(body)
}

/// The inverse of `encode_relay_record`; the record length must match and
/// stay within the limit of its codec.
pub fn decode_relay_record(payload: &[u8]) -> std::io::Result<(GopHeader, FrameRecord)> {
    let invalid = |text: &'static str| std::io::Error::new(std::io::ErrorKind::InvalidData, text);
    let (head, rest) = payload
        .split_at_checked(GOP_HEADER_BYTES)
        .ok_or_else(|| invalid("screen relay record is too short"))?;
    let mut bytes = [0u8; GOP_HEADER_BYTES];
    bytes.copy_from_slice(head);
    let (mut header, extended) = GopHeader::decode_prefix(&bytes)?;
    let rest = if extended {
        let (length, data) = rest
            .split_at_checked(2)
            .ok_or_else(|| invalid("screen relay contract length is missing"))?;
        let length = u16::from_be_bytes([length[0], length[1]]) as usize;
        if length > 1024 {
            return Err(invalid("screen relay contract exceeds 1 KiB"));
        }
        let (json, remaining) = data
            .split_at_checked(length)
            .ok_or_else(|| invalid("screen relay contract is truncated"))?;
        let extension: RelayGopExtension =
            serde_json::from_slice(json).map_err(std::io::Error::other)?;
        header.frame_contract = Some(extension.contract);
        header.family_layout_revision = extension.family_layout_revision;
        // Reuse the header's full validation, including a nonzero family
        // revision bound to a v2 contract, without altering the received values.
        header.encode()?;
        remaining
    } else {
        rest
    };
    let (fields, data) = rest
        .split_at_checked(RECORD_HEADER_BYTES)
        .ok_or_else(|| invalid("screen relay record is too short"))?;
    let mut reader = BeReader::new(fields);
    let len = reader.u32()? as usize;
    let frame_id = reader.u64()?;
    let capture_ts_us = reader.u64()?;
    let flags = reader.u8()?;
    if len != data.len() || len > header.record_limit() {
        return Err(invalid("screen relay record length does not match"));
    }
    let record = FrameRecord {
        frame_id,
        capture_ts_us,
        flags,
        payload: data.to_vec(),
    };
    Ok((header, record))
}

/// Writes one relay frame.
pub fn write_relay(
    writer: &mut impl std::io::Write,
    tag: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::other("screen relay frame is too large"))?;
    writer.write_all(&[tag])?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

/// Reads one relay frame of at most `limit` payload bytes; the length is
/// checked before memory for it is requested.
pub fn read_relay(reader: &mut impl std::io::Read, limit: usize) -> std::io::Result<(u8, Vec<u8>)> {
    read_relay_budgeted(reader, limit, limit)
}

/// Picture records have the accepted session's tighter allocation ceiling;
/// control frames retain their existing independent protocol limit.
pub fn read_relay_budgeted(
    reader: &mut impl std::io::Read,
    limit: usize,
    record_limit: usize,
) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    reader.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit || (head[0] == RELAY_RECORD && len > record_limit) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "screen relay frame exceeds its limit",
        ));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok((head[0], payload))
}
