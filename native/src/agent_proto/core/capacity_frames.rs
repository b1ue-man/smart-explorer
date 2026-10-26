//! capacity-v1 metadata frames; payload is a bounded serde DTO at the host edge.
use super::super::super::types::Frame;
use super::super::{bad, Reader};
use super::{put_str, string_len};
use std::io;
const MAX_ROOT_BYTES: usize = 32_768;
// Each permitted root/identity byte may require a six-byte JSON escape;
// fixed DTO fields, enum names and decimal u64 values fit in the remainder.
const MAX_JSON_BYTES: usize = 6 * (MAX_ROOT_BYTES + 4096) + 4096;
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    match frame {
        Frame::Capacity(value) if value.len() <= MAX_ROOT_BYTES => Some(string_len(value)),
        Frame::CapacityData(value) if value.len() <= MAX_JSON_BYTES => Some(string_len(value)),
        Frame::Capacity(_) | Frame::CapacityData(_) => {
            Some(Err(bad("capacity metadata exceeds protocol limit")))
        }
        _ => None,
    }
}
pub(in super::super) fn encode(frame: &Frame, bytes: &mut Vec<u8>) -> bool {
    match frame {
        Frame::Capacity(path) => {
            bytes.push(70);
            put_str(bytes, path);
        }
        Frame::CapacityData(value) => {
            bytes.push(71);
            put_str(bytes, value);
        }
        _ => return false,
    }
    true
}
pub(in super::super) fn decode(tag: u8, reader: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(match tag {
        70 => Some(Frame::Capacity(reader.tree_string(MAX_ROOT_BYTES)?)),
        71 => Some(Frame::CapacityData(reader.tree_string(MAX_JSON_BYTES)?)),
        _ => None,
    })
}
