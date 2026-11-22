//! Large control frames in parts (review round 10, Codex 10/10). Under a
//! user ceiling the host sends a large control frame (window list, icon,
//! family state up to 1 MiB) piece by piece within the byte budget; on one
//! ordered stream every small frame (`CONTROL_STATE`, `ERROR`, `STATE`, a
//! `PONG`) waited behind its last piece, at low rates for many seconds. A
//! viewer that offers `control_parts` (`ScreenClientHello`) takes each piece
//! as a frame of its own, `CONTROL_PART`, so small frames go between them;
//! it puts the large frame together again (`PartAssembler`) before it
//! decodes it. Parts of one frame come in order and never interleave with
//! the parts of another.
//!
//! `CONTROL_PART` payload: the inner frame's tag (1 byte), its payload's
//! full length (u32 BE), the offset of this part in it (u32 BE), the bytes.
use std::io;

use super::wire::{invalid, tag_limit};

/// Host to viewer: one part of a large control frame.
pub const CONTROL_PART: u8 = 56;
/// The version a viewer offers in `ScreenClientHello::control_parts`.
pub const CONTROL_PARTS_V1: u16 = 1;
/// The head of a part's payload.
pub const PART_HEAD: usize = 9;

/// The payload of the part of `inner_payload` (of `inner_tag`) at `offset`.
pub fn encode_part(inner_tag: u8, inner_payload: &[u8], offset: usize, len: usize) -> Vec<u8> {
    let total = u32::try_from(inner_payload.len()).unwrap_or(u32::MAX);
    let at = u32::try_from(offset).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(PART_HEAD + len);
    out.push(inner_tag);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&at.to_be_bytes());
    out.extend_from_slice(&inner_payload[offset..offset + len]);
    out
}

/// The viewer's large frame being put together.
#[derive(Default)]
pub struct PartAssembler {
    building: Option<(u8, usize, Vec<u8>)>,
}

impl PartAssembler {
    /// One `CONTROL_PART`; the inner frame's tag and payload once whole.
    /// A part out of order, beyond its frame or beyond its tag's limit ends
    /// the stream as malformed.
    pub fn push(&mut self, part: &[u8]) -> io::Result<Option<(u8, Vec<u8>)>> {
        if part.len() < PART_HEAD {
            return Err(invalid("control part is too short"));
        }
        let tag = part[0];
        let total = u32::from_be_bytes([part[1], part[2], part[3], part[4]]) as usize;
        let offset = u32::from_be_bytes([part[5], part[6], part[7], part[8]]) as usize;
        let bytes = &part[PART_HEAD..];
        if tag == CONTROL_PART || total > tag_limit(tag) {
            return Err(invalid("control part exceeds the limit of its frame"));
        }
        let (built_tag, built_total, buffer) = match self.building.as_mut() {
            Some(building) => building,
            None if offset == 0 => self.building.insert((tag, total, Vec::new())),
            None => return Err(invalid("control part out of order")),
        };
        if *built_tag != tag || *built_total != total || buffer.len() != offset {
            return Err(invalid("control part out of order"));
        }
        if offset + bytes.len() > total {
            return Err(invalid("control part beyond its frame"));
        }
        buffer.extend_from_slice(bytes);
        if buffer.len() < total {
            return Ok(None);
        }
        Ok(self.building.take().map(|(tag, _, payload)| (tag, payload)))
    }
}

#[cfg(test)]
#[path = "control_parts_tests.rs"]
mod tests;
