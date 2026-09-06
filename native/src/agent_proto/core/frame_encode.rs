//! Exact-size frame encoding, with optional space for the transport length.
use std::io;

use super::super::{
    node_codec::validate_node,
    relative_path::ValidatedRelativePath,
    types::{Frame, WireMeta, WireNode, CHUNK},
};
use super::{bad, validate_frame_len, MIN_WIRE_META_BYTES};

impl Frame {
    pub fn encode(&self, req_id: u64) -> io::Result<Vec<u8>> {
        encode(self, req_id, false)
    }
}

/// Allocate the final buffer once; never prepend by copying an encoded body.
pub(super) fn encode(frame: &Frame, req_id: u64, with_length: bool) -> io::Result<Vec<u8>> {
    let body_len = encoded_len(frame)?;
    let prefix = if with_length { 4 } else { 0 };
    let capacity = body_len
        .checked_add(prefix)
        .ok_or_else(|| bad("frame too large"))?;
    let mut b = Vec::with_capacity(capacity);
    b.resize(prefix, 0);
    put_u64(&mut b, req_id);
    match frame {
        Frame::Hello { proto } => {
            b.push(1);
            put_u32(&mut b, *proto);
        }
        Frame::HelloOk { proto, version } => {
            b.push(2);
            put_u32(&mut b, *proto);
            put_str(&mut b, version);
        }
        Frame::ListDir(p) => {
            b.push(3);
            put_str(&mut b, p);
        }
        Frame::Dir(v) => {
            b.push(4);
            put_u32(&mut b, v.len() as u32);
            for m in v {
                put_meta(&mut b, m);
            }
        }
        Frame::Stat(p) => {
            b.push(5);
            put_str(&mut b, p);
        }
        Frame::Meta(m) => {
            b.push(6);
            put_meta(&mut b, m);
        }
        Frame::WalkTree(p) => {
            b.push(7);
            put_str(&mut b, p);
        }
        Frame::Tree(n) => {
            b.push(8);
            put_node(&mut b, n);
        }
        Frame::Read { path, offset, len } => {
            b.push(9);
            put_str(&mut b, path);
            put_u64(&mut b, *offset);
            put_u64(&mut b, *len);
        }
        Frame::Write(p) => put_tagged_str(&mut b, 10, p),
        Frame::WriteNew(p) => put_tagged_str(&mut b, 33, p),
        Frame::Data(d) => {
            b.push(11);
            put_bytes(&mut b, d);
        }
        Frame::Copy { src, dst } => {
            b.push(12);
            put_str(&mut b, src);
            put_str(&mut b, dst);
        }
        Frame::Rename { src, dst } => {
            b.push(13);
            put_str(&mut b, src);
            put_str(&mut b, dst);
        }
        Frame::Remove { path, recursive } => {
            b.push(14);
            put_str(&mut b, path);
            put_bool(&mut b, *recursive);
        }
        Frame::Mkdir(p) => {
            b.push(15);
            put_str(&mut b, p);
        }
        Frame::GetTree(p) => {
            b.push(16);
            put_str(&mut b, p);
        }
        Frame::PutTree(p) => {
            b.push(17);
            put_str(&mut b, p);
        }
        Frame::TreeEntry {
            rel,
            is_dir,
            size,
            mtime_ms,
        } => {
            b.push(18);
            put_str(&mut b, rel);
            put_bool(&mut b, *is_dir);
            put_u64(&mut b, *size);
            put_i64(&mut b, *mtime_ms);
        }
        Frame::Search { root, spec } => {
            b.push(19);
            put_str(&mut b, root);
            put_str(&mut b, &spec.query);
            put_bool(&mut b, spec.glob);
            put_u64(&mut b, spec.min_size);
            put_u64(&mut b, spec.max_size);
            put_u64(&mut b, spec.max_results);
            put_bool(&mut b, spec.want_dirs);
        }
        Frame::Match {
            rel,
            is_dir,
            size,
            mtime_ms,
        } => {
            b.push(20);
            put_str(&mut b, rel);
            put_bool(&mut b, *is_dir);
            put_u64(&mut b, *size);
            put_i64(&mut b, *mtime_ms);
        }
        Frame::WalkHashed { root, want_hash } => {
            b.push(21);
            put_str(&mut b, root);
            put_bool(&mut b, *want_hash);
        }
        Frame::HashEntry {
            rel,
            is_dir,
            size,
            mtime_ms,
            md5,
        } => {
            b.push(22);
            put_str(&mut b, rel);
            put_bool(&mut b, *is_dir);
            put_u64(&mut b, *size);
            put_i64(&mut b, *mtime_ms);
            put_opt_str(&mut b, md5);
        }
        Frame::Progress { done, total } => {
            b.push(23);
            put_u64(&mut b, *done);
            put_u64(&mut b, *total);
        }
        Frame::Ok => b.push(24),
        Frame::End => b.push(25),
        Frame::Err(e) => {
            b.push(26);
            put_str(&mut b, e);
        }
        Frame::Cancel => b.push(27),
        Frame::TryExists(p) => put_tagged_str(&mut b, 28, p),
        Frame::Exists(exists) => {
            b.push(29);
            put_bool(&mut b, *exists);
        }
        Frame::RenameNoReplace { src, dst } => {
            b.push(30);
            put_str(&mut b, src);
            put_str(&mut b, dst);
        }
        Frame::Promote {
            staged,
            destination,
        } => {
            b.push(31);
            put_str(&mut b, staged);
            put_str(&mut b, destination);
        }
        Frame::PromoteNoReplace {
            staged,
            destination,
        } => {
            b.push(32);
            put_str(&mut b, staged);
            put_str(&mut b, destination);
        }
    }
    if b.len() != capacity {
        return Err(bad("frame size calculation mismatch"));
    }
    if with_length {
        // encoded_len checked the 64-MiB body boundary before allocation.
        b[..4].copy_from_slice(&(body_len as u32).to_le_bytes());
    }
    Ok(b)
}

fn add_len(total: &mut usize, extra: usize) -> io::Result<()> {
    *total = total
        .checked_add(extra)
        .ok_or_else(|| bad("frame too large"))?;
    validate_frame_len(*total)
}

fn string_len(value: &str) -> io::Result<usize> {
    u32::try_from(value.len()).map_err(|_| bad("frame too large"))?;
    let mut length = 4;
    add_len(&mut length, value.len())?;
    Ok(length)
}

fn optional_string_len(value: &Option<String>) -> io::Result<usize> {
    let mut length = 1;
    if let Some(value) = value {
        add_len(&mut length, string_len(value)?)?;
    }
    Ok(length)
}

fn metadata_len(metadata: &WireMeta) -> io::Result<usize> {
    // name prefix + two flags + size + mtime + optional-string flag.
    let mut length = MIN_WIRE_META_BYTES;
    u32::try_from(metadata.name.len()).map_err(|_| bad("frame too large"))?;
    add_len(&mut length, metadata.name.len())?;
    if let Some(md5) = &metadata.content_md5 {
        add_len(&mut length, string_len(md5)?)?;
    }
    Ok(length)
}

fn directory_len(entries: &[WireMeta]) -> io::Result<usize> {
    u32::try_from(entries.len()).map_err(|_| bad("frame too large"))?;
    // Request id, Dir tag and entry count occupy thirteen body bytes.
    let mut minimum = 13;
    add_len(
        &mut minimum,
        entries
            .len()
            .checked_mul(MIN_WIRE_META_BYTES)
            .ok_or_else(|| bad("frame too large"))?,
    )?;
    let mut length = 13;
    for metadata in entries {
        add_len(&mut length, metadata_len(metadata)?)?;
    }
    Ok(length)
}

fn node_len(node: &WireNode) -> io::Result<usize> {
    u32::try_from(node.children.len()).map_err(|_| bad("frame too large"))?;
    let mut length = string_len(&node.name)?;
    add_len(&mut length, 13)?; // size, directory flag, child count
    for child in &node.children {
        add_len(&mut length, node_len(child)?)?;
    }
    Ok(length)
}

/// Validate lengths/counts before constructing the outbound byte buffer.
fn encoded_len(frame: &Frame) -> io::Result<usize> {
    let mut length = 9; // request id + tag
    match frame {
        Frame::Hello { .. } => add_len(&mut length, 4)?,
        Frame::HelloOk { version, .. } => {
            add_len(&mut length, 4)?;
            add_len(&mut length, string_len(version)?)?;
        }
        Frame::ListDir(path)
        | Frame::Stat(path)
        | Frame::WalkTree(path)
        | Frame::Write(path)
        | Frame::WriteNew(path)
        | Frame::Mkdir(path)
        | Frame::GetTree(path)
        | Frame::PutTree(path)
        | Frame::Err(path)
        | Frame::TryExists(path) => add_len(&mut length, string_len(path)?)?,
        Frame::Dir(entries) => return directory_len(entries),
        Frame::Meta(metadata) => add_len(&mut length, metadata_len(metadata)?)?,
        Frame::Tree(node) => {
            validate_node(node)?;
            add_len(&mut length, node_len(node)?)?;
        }
        Frame::Read { path, .. } => {
            add_len(&mut length, string_len(path)?)?;
            add_len(&mut length, 16)?;
        }
        Frame::Data(data) => {
            if data.len() > CHUNK {
                return Err(bad("data frame exceeds the protocol chunk size"));
            }
            add_len(&mut length, 4 + data.len())?;
        }
        Frame::Copy { src, dst }
        | Frame::Rename { src, dst }
        | Frame::RenameNoReplace { src, dst }
        | Frame::Promote {
            staged: src,
            destination: dst,
        }
        | Frame::PromoteNoReplace {
            staged: src,
            destination: dst,
        } => {
            add_len(&mut length, string_len(src)?)?;
            add_len(&mut length, string_len(dst)?)?;
        }
        Frame::Remove { path, .. } => {
            add_len(&mut length, string_len(path)?)?;
            add_len(&mut length, 1)?;
        }
        Frame::TreeEntry { rel, .. } | Frame::Match { rel, .. } => {
            if matches!(frame, Frame::TreeEntry { .. }) {
                ValidatedRelativePath::parse(rel)?;
            }
            add_len(&mut length, string_len(rel)?)?;
            add_len(&mut length, 17)?;
        }
        Frame::Search { root, spec } => {
            add_len(&mut length, string_len(root)?)?;
            add_len(&mut length, string_len(&spec.query)?)?;
            add_len(&mut length, 26)?;
        }
        Frame::WalkHashed { root, .. } => {
            add_len(&mut length, string_len(root)?)?;
            add_len(&mut length, 1)?;
        }
        Frame::HashEntry { rel, md5, .. } => {
            add_len(&mut length, string_len(rel)?)?;
            add_len(&mut length, 17)?;
            add_len(&mut length, optional_string_len(md5)?)?;
        }
        Frame::Progress { .. } => add_len(&mut length, 16)?,
        Frame::Exists(_) => add_len(&mut length, 1)?,
        Frame::Ok | Frame::End | Frame::Cancel => {}
    }
    Ok(length)
}

// Every narrowed length/count below has passed encoded_len's checked preflight.
fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_i64(b: &mut Vec<u8>, v: i64) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn put_bool(b: &mut Vec<u8>, v: bool) {
    b.push(v as u8);
}
fn put_str(b: &mut Vec<u8>, s: &str) {
    put_u32(b, s.len() as u32);
    b.extend_from_slice(s.as_bytes());
}
fn put_tagged_str(b: &mut Vec<u8>, tag: u8, value: &str) {
    b.push(tag);
    put_str(b, value);
}
fn put_bytes(b: &mut Vec<u8>, s: &[u8]) {
    put_u32(b, s.len() as u32);
    b.extend_from_slice(s);
}

fn put_opt_str(b: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(v) => {
            put_bool(b, true);
            put_str(b, v);
        }
        None => put_bool(b, false),
    }
}

fn put_meta(b: &mut Vec<u8>, m: &WireMeta) {
    put_str(b, &m.name);
    put_bool(b, m.is_dir);
    put_bool(b, m.is_symlink);
    put_u64(b, m.size);
    put_i64(b, m.mtime_ms);
    put_opt_str(b, &m.content_md5);
}

fn put_node(b: &mut Vec<u8>, n: &WireNode) {
    put_str(b, &n.name);
    put_u64(b, n.size);
    put_bool(b, n.is_dir);
    put_u32(b, n.children.len() as u32);
    for c in &n.children {
        put_node(b, c);
    }
}
