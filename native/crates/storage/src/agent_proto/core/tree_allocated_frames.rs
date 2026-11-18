//! `tree-allocated-v1`: `WalkTreeAllocated` (tag 91) asks for the size tree
//! with every node's occupied bytes, `TreeAllocated` (tag 92) carries it.
//! A record is the `Tree` record with the occupied bytes after the size
//! (`u64::MAX`: not known). The reply tree is decoded in `codec.rs`, where
//! the receiver's tree budget applies.
use super::super::super::node_codec::{validate_node, UNKNOWN_ALLOCATED};
use super::super::super::types::{Frame, WireNode};
use super::super::bad;
use super::{add_len, put_bool, put_str, put_u32, put_u64, string_len};
use std::io;

pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    match frame {
        Frame::WalkTreeAllocated(path) => Some(string_len(path)),
        Frame::TreeAllocated(node) => Some(validate_node(node).and_then(|()| node_len(node))),
        _ => None,
    }
}

pub(in super::super) fn encode(frame: &Frame, bytes: &mut Vec<u8>) -> bool {
    match frame {
        Frame::WalkTreeAllocated(path) => {
            bytes.push(91);
            put_str(bytes, path);
        }
        Frame::TreeAllocated(node) => {
            bytes.push(92);
            put_node(bytes, node);
        }
        _ => return false,
    }
    true
}

fn node_len(node: &WireNode) -> io::Result<usize> {
    u32::try_from(node.children.len()).map_err(|_| bad("frame too large"))?;
    let mut length = string_len(&node.name)?;
    add_len(&mut length, 21)?; // size, occupied bytes, directory flag, child count
    for child in &node.children {
        add_len(&mut length, node_len(child)?)?;
    }
    Ok(length)
}

fn put_node(bytes: &mut Vec<u8>, node: &WireNode) {
    put_str(bytes, &node.name);
    put_u64(bytes, node.size);
    put_u64(bytes, node.allocated.unwrap_or(UNKNOWN_ALLOCATED));
    put_bool(bytes, node.is_dir);
    put_u32(bytes, node.children.len() as u32);
    for child in &node.children {
        put_node(bytes, child);
    }
}
