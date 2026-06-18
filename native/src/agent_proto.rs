//! Shared, dependency-free (std + rayon only) core for the SSH remote agent —
//! the wire PROTOCOL (v2) and the LOCAL filesystem operations it runs
//! server-side.
//!
//! Included by BOTH the app (the `AgentBackend` transport) and the `se-agent`
//! binary, so there is exactly one definition of the frames and the walk, and
//! the agent binary pulls in nothing else (no vfs/analytics/GUI/TLS) → it stays
//! tiny and cross-compiles cleanly to static musl. See `docs/SSH_AGENT_PLAN.md`.
//!
//! ## Protocol v2 — multiplexed + streaming
//!
//! Framing: each frame is `u32 LE length` + that many body bytes; the body is
//! `u64 LE req_id` + `u8 tag` + a compact hand-rolled binary payload (no
//! serde_json — a million-node `WalkTree` response must not pay JSON's cost).
//!
//! Every frame carries a `req_id`, so many operations share ONE channel: the
//! client tags each request with a fresh id and routes replies/stream-chunks
//! back to the waiting op by id (the bridge worker in `agent.rs`). Streaming ops
//! (read/write/get-tree/put-tree/search/walk-hashed) emit a sequence of frames
//! under one id terminated by `End` (or `Err`); `Cancel{req_id}` aborts one.
//!
//! The agent serves requests concurrently (one thread per request) and serial-
//! ises stdout writes behind a mutex, so a slow transfer never blocks a quick
//! `list_dir` — frames interleave on the wire.
#![allow(dead_code)]

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Bumped whenever the wire format OR the agent's behaviour changes; the client
/// re-uploads the agent on a mismatch (handshake in `Hello`/`HelloOk`, and the
/// install path is keyed on this).
pub const PROTO_VERSION: u32 = 4;

/// Reject absurd frame lengths from a corrupt/hostile stream before allocating.
const MAX_FRAME: usize = 1 << 31; // 2 GiB

/// Payload chunk size for streamed byte transfers (read/write/tree). A few
/// hundred KiB amortises framing overhead without inflating latency.
pub const CHUNK: usize = 256 * 1024;

/// Backend-neutral directory entry (a subset of `vfs::VfsMeta` — the fields a
/// local `std::fs` listing can supply cheaply).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

/// One node of the size tree (mirrors `analytics::SizeNode`: own name, recursive
/// size, children; empty children for files).
#[derive(Clone, Debug, PartialEq)]
pub struct WireNode {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<WireNode>,
}

/// A server-side search request (Phase 5). `query` is matched case-insensitively
/// against each entry's name as a substring (or a `*?`-glob when `glob`).
#[derive(Clone, Debug, PartialEq)]
pub struct SearchSpec {
    pub query: String,
    pub glob: bool,
    pub min_size: u64,
    /// 0 = no upper bound.
    pub max_size: u64,
    /// 0 = unlimited.
    pub max_results: u64,
    /// Match directories too (else only files).
    pub want_dirs: bool,
}

/// One frame on the wire. Requests, responses and stream chunks all ride the
/// same enum so the channel is fully bidirectional (e.g. an upload sends `Write`
/// then a run of `Data` then `End`, all under one `req_id`).
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    // ── handshake ──
    Hello { proto: u32 },
    HelloOk { proto: u32, version: String },
    // ── metadata (request → single response) ──
    ListDir(String),
    Dir(Vec<WireMeta>),
    Stat(String),
    Meta(WireMeta),
    WalkTree(String),
    Tree(WireNode),
    // ── byte streams ──
    /// Read `len` bytes from `offset` (len 0 = to EOF) → `Data`* `End`.
    Read { path: String, offset: u64, len: u64 },
    /// Begin writing `path`; client follows with `Data`* `End` → `Ok`.
    Write(String),
    /// A chunk of a byte stream (either direction).
    Data(Vec<u8>),
    // ── server-local mutations (request → Ok/Err) ──
    Copy { src: String, dst: String },
    Rename { src: String, dst: String },
    Remove { path: String, recursive: bool },
    Mkdir(String),
    // ── recursive bulk transfer (Phase 4) ──
    /// Stream an entire subtree down: `TreeEntry`(+`Data`* for files)… `End`.
    GetTree(String),
    /// Receive an entire subtree: client streams `TreeEntry`(+`Data`*)… `End`.
    PutTree(String),
    /// Header for one entry inside a Get/PutTree stream (path RELATIVE to root).
    TreeEntry { rel: String, is_dir: bool, size: u64, mtime_ms: i64 },
    // ── server-side search (Phase 5) ──
    Search { root: String, spec: SearchSpec },
    Match { rel: String, is_dir: bool, size: u64, mtime_ms: i64 },
    // ── hashing / sync (Phase 6) ──
    WalkHashed { root: String, want_hash: bool },
    HashEntry { rel: String, is_dir: bool, size: u64, mtime_ms: i64, md5: Option<String> },
    // ── generic ──
    Progress { done: u64, total: u64 },
    Ok,
    End,
    Err(String),
    Cancel,
}

// ── encoding primitives ──────────────────────────────────────────────────────

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

/// Cursor over a frame body.
struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.i + n > self.b.len() {
            return Err(bad("truncated frame"));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> io::Result<bool> {
        Ok(self.u8()? != 0)
    }
    fn string(&mut self) -> io::Result<String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| bad("invalid utf8"))
    }
    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn opt_str(&mut self) -> io::Result<Option<String>> {
        Ok(if self.bool()? { Some(self.string()?) } else { None })
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn put_meta(b: &mut Vec<u8>, m: &WireMeta) {
    put_str(b, &m.name);
    put_bool(b, m.is_dir);
    put_bool(b, m.is_symlink);
    put_u64(b, m.size);
    put_i64(b, m.mtime_ms);
}
fn get_meta(r: &mut Reader) -> io::Result<WireMeta> {
    Ok(WireMeta {
        name: r.string()?,
        is_dir: r.bool()?,
        is_symlink: r.bool()?,
        size: r.u64()?,
        mtime_ms: r.i64()?,
    })
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
fn get_node(r: &mut Reader) -> io::Result<WireNode> {
    let name = r.string()?;
    let size = r.u64()?;
    let is_dir = r.bool()?;
    let n = r.u32()? as usize;
    let mut children = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        children.push(get_node(r)?);
    }
    Ok(WireNode { name, size, is_dir, children })
}

impl Frame {
    pub fn encode(&self, req_id: u64) -> Vec<u8> {
        let mut b = Vec::new();
        put_u64(&mut b, req_id);
        match self {
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
            Frame::Write(p) => {
                b.push(10);
                put_str(&mut b, p);
            }
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
            Frame::TreeEntry { rel, is_dir, size, mtime_ms } => {
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
            Frame::Match { rel, is_dir, size, mtime_ms } => {
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
            Frame::HashEntry { rel, is_dir, size, mtime_ms, md5 } => {
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
        }
        b
    }

    /// Decode a frame body → `(req_id, frame)`.
    pub fn decode(body: &[u8]) -> io::Result<(u64, Frame)> {
        let mut r = Reader::new(body);
        let req_id = r.u64()?;
        let frame = match r.u8()? {
            1 => Frame::Hello { proto: r.u32()? },
            2 => Frame::HelloOk { proto: r.u32()?, version: r.string()? },
            3 => Frame::ListDir(r.string()?),
            4 => {
                let n = r.u32()? as usize;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(get_meta(&mut r)?);
                }
                Frame::Dir(v)
            }
            5 => Frame::Stat(r.string()?),
            6 => Frame::Meta(get_meta(&mut r)?),
            7 => Frame::WalkTree(r.string()?),
            8 => Frame::Tree(get_node(&mut r)?),
            9 => Frame::Read { path: r.string()?, offset: r.u64()?, len: r.u64()? },
            10 => Frame::Write(r.string()?),
            11 => Frame::Data(r.bytes()?),
            12 => Frame::Copy { src: r.string()?, dst: r.string()? },
            13 => Frame::Rename { src: r.string()?, dst: r.string()? },
            14 => Frame::Remove { path: r.string()?, recursive: r.bool()? },
            15 => Frame::Mkdir(r.string()?),
            16 => Frame::GetTree(r.string()?),
            17 => Frame::PutTree(r.string()?),
            18 => Frame::TreeEntry {
                rel: r.string()?,
                is_dir: r.bool()?,
                size: r.u64()?,
                mtime_ms: r.i64()?,
            },
            19 => Frame::Search {
                root: r.string()?,
                spec: SearchSpec {
                    query: r.string()?,
                    glob: r.bool()?,
                    min_size: r.u64()?,
                    max_size: r.u64()?,
                    max_results: r.u64()?,
                    want_dirs: r.bool()?,
                },
            },
            20 => Frame::Match {
                rel: r.string()?,
                is_dir: r.bool()?,
                size: r.u64()?,
                mtime_ms: r.i64()?,
            },
            21 => Frame::WalkHashed { root: r.string()?, want_hash: r.bool()? },
            22 => Frame::HashEntry {
                rel: r.string()?,
                is_dir: r.bool()?,
                size: r.u64()?,
                mtime_ms: r.i64()?,
                md5: r.opt_str()?,
            },
            23 => Frame::Progress { done: r.u64()?, total: r.u64()? },
            24 => Frame::Ok,
            25 => Frame::End,
            26 => Frame::Err(r.string()?),
            27 => Frame::Cancel,
            t => return Err(bad(&format!("unknown frame tag {t}"))),
        };
        Ok((req_id, frame))
    }
}

// ── framing ──────────────────────────────────────────────────────────────────

/// Write a length-prefixed frame and flush.
pub fn write_frame(w: &mut impl Write, req_id: u64, frame: &Frame) -> io::Result<()> {
    let body = frame.encode(req_id);
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Read one frame. `Ok(None)` = clean EOF before any byte of the next frame.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u64, Frame)>> {
    let mut lenb = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut lenb[got..])? {
            0 if got == 0 => return Ok(None),
            0 => return Err(bad("eof inside length")),
            n => got += n,
        }
    }
    let len = u32::from_le_bytes(lenb) as usize;
    if len > MAX_FRAME {
        return Err(bad("frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(Frame::decode(&body)?))
}

// ── local filesystem operations (run server-side by the agent) ───────────────

/// Linux pseudo-filesystems whose "files" report bogus huge sizes (e.g.
/// `/proc/kcore` ≈ the 128 TiB virtual address space). A size walk must skip
/// them or totals explode. No-op for Windows-style paths (they never match).
pub fn is_pseudo_dir(path: &str) -> bool {
    let p = path.trim_end_matches('/');
    matches!(p, "/proc" | "/sys" | "/dev" | "/run")
        || p.starts_with("/proc/")
        || p.starts_with("/sys/")
        || p.starts_with("/dev/")
        || p.starts_with("/run/")
}

fn systemtime_ms(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// List one directory's entries (names only — no recursion).
pub fn list_local(path: &str) -> io::Result<Vec<WireMeta>> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(path)? {
        let ent = ent?;
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let md = ent.metadata().ok();
        out.push(WireMeta {
            name: ent.file_name().to_string_lossy().into_owned(),
            is_dir: ft.is_dir(),
            is_symlink: ft.is_symlink(),
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime_ms: md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(systemtime_ms)
                .unwrap_or(0),
        });
    }
    Ok(out)
}

/// Metadata for a single path.
pub fn stat_local(path: &str) -> io::Result<WireMeta> {
    let p = Path::new(path);
    let md = std::fs::symlink_metadata(p)?;
    let ft = md.file_type();
    Ok(WireMeta {
        name: p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        is_dir: md.is_dir(),
        is_symlink: ft.is_symlink(),
        size: md.len(),
        mtime_ms: md.modified().ok().map(systemtime_ms).unwrap_or(0),
    })
}

/// Recursive size walk, run locally on the server. Parallel like
/// `analytics::scan` (the server has cores; this is the headline win — the whole
/// tree is computed in one shot and returned, no per-dir round-trip).
pub fn walk_local(root: &Path) -> WireNode {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    walk_dir(root, name)
}

fn walk_dir(dir: &Path, name: String) -> WireNode {
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue; // don't follow — avoids cycles + double counting
            }
            let nm = ent.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                let cp = ent.path();
                if is_pseudo_dir(&cp.to_string_lossy()) {
                    continue; // /proc, /sys, … — bogus sizes
                }
                subdirs.push((cp, nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own += sz;
                files.push(WireNode { name: nm, size: sz, is_dir: false, children: Vec::new() });
            }
        }
    }
    let mut dir_nodes: Vec<WireNode> = if subdirs.len() > 1 {
        subdirs.into_par_iter().map(|(p, n)| walk_dir(&p, n)).collect()
    } else {
        subdirs.into_iter().map(|(p, n)| walk_dir(&p, n)).collect()
    };
    let mut size = own;
    for d in &dir_nodes {
        size += d.size;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    WireNode { name, size, is_dir: true, children }
}

/// Live counters for a `WalkTree` (so the client can show progress while the
/// server walks): running file count + byte total.
pub struct WalkCounter {
    pub files: std::sync::atomic::AtomicU64,
    pub bytes: std::sync::atomic::AtomicU64,
}

/// Recursive size walk that updates `cnt` live and bails on `cancel` (returning
/// the partial subtree — the client discards a cancelled walk anyway).
fn walk_dir_counted(dir: &Path, name: String, cnt: &WalkCounter, cancel: &AtomicBool) -> WireNode {
    if cancel.load(Ordering::Relaxed) {
        return WireNode { name, size: 0, is_dir: true, children: Vec::new() };
    }
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let nm = ent.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                let cp = ent.path();
                if is_pseudo_dir(&cp.to_string_lossy()) {
                    continue;
                }
                subdirs.push((cp, nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own += sz;
                cnt.files.fetch_add(1, Ordering::Relaxed);
                cnt.bytes.fetch_add(sz, Ordering::Relaxed);
                files.push(WireNode { name: nm, size: sz, is_dir: false, children: Vec::new() });
            }
        }
    }
    let mut dir_nodes: Vec<WireNode> = if subdirs.len() > 1 {
        subdirs.into_par_iter().map(|(p, n)| walk_dir_counted(&p, n, cnt, cancel)).collect()
    } else {
        subdirs.into_iter().map(|(p, n)| walk_dir_counted(&p, n, cnt, cancel)).collect()
    };
    let mut size = own;
    for d in &dir_nodes {
        size += d.size;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    WireNode { name, size, is_dir: true, children }
}

/// Walk `root` server-side, emitting periodic `Progress{files, bytes}` frames
/// while it runs, then the final `Tree`. Respects `cancel`.
fn handle_walk_tree(sink: &Sink, id: u64, root: &str, cancel: &AtomicBool) -> io::Result<()> {
    let p = Path::new(root);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string());
    let cnt = Arc::new(WalkCounter {
        files: std::sync::atomic::AtomicU64::new(0),
        bytes: std::sync::atomic::AtomicU64::new(0),
    });
    let done = Arc::new(AtomicBool::new(false));
    // Progress emitter: ~5/s while the walk runs.
    let sink2 = sink.clone();
    let cnt2 = cnt.clone();
    let done2 = done.clone();
    let emitter = std::thread::spawn(move || {
        while !done2.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let f = cnt2.files.load(Ordering::Relaxed);
            let b = cnt2.bytes.load(Ordering::Relaxed);
            if emit(&sink2, id, &Frame::Progress { done: f, total: b }).is_err() {
                break;
            }
        }
    });
    let tree = walk_dir_counted(p, name, &cnt, cancel);
    done.store(true, Ordering::Relaxed);
    let _ = emitter.join();
    emit(sink, id, &Frame::Tree(tree))
}

// ── server-side streaming handlers ───────────────────────────────────────────

/// A shared, mutex-guarded frame sink. Per-frame locking lets a quick op's reply
/// interleave between a transfer's chunks rather than waiting for it to finish.
type Sink = Arc<Mutex<Box<dyn Write + Send>>>;

fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "sink poisoned"))?;
    write_frame(&mut *w, id, frame)
}

/// Read a file `[offset, offset+len)` (len 0 = to EOF) → `Data`* then `End`.
fn handle_read(
    sink: &Sink,
    id: u64,
    path: &str,
    offset: u64,
    len: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset))?;
    }
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(()); // abandon quietly; client already moved on
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
        remaining -= n as u64;
    }
    emit(sink, id, &Frame::End)
}

/// Receive a byte stream (`Data`* `End`) into `path` via a temp + atomic rename.
fn handle_write(
    sink: &Sink,
    id: u64,
    path: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let tmp = format!("{path}.se-agent.part");
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(&tmp)?;
    // Positive "ready" ack: the temp file was created, so the client may start
    // streaming. Lets `open_write` fail fast (and fall back to SFTP) on a path/
    // permission error instead of discovering it only at close. A `Data`-less
    // `Progress{0,0}` is unambiguous here (writes never report mid-progress).
    emit(sink, id, &Frame::Progress { done: 0, total: 0 })?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::Data(d)) => f.write_all(&d)?,
            Ok(Frame::End) => break,
            Ok(_) => {} // ignore stray frames
            Err(_) => {
                // client vanished mid-upload → discard the partial file
                drop(f);
                let _ = std::fs::remove_file(&tmp);
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "upload aborted"));
            }
        }
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    emit(sink, id, &Frame::Ok)
}

fn remove_path(path: &str, recursive: bool) -> io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_dir() {
        if recursive {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_dir(path)
        }
    } else {
        std::fs::remove_file(path)
    }
}

/// Stream an entire subtree down: dirs as `TreeEntry`, files as `TreeEntry` +
/// `Data`* , finished by `End`. Depth-first, paths RELATIVE to `root`.
fn handle_get_tree(sink: &Sink, id: u64, root: &str, cancel: &AtomicBool) -> io::Result<()> {
    fn walk(sink: &Sink, id: u64, base: &Path, dir: &Path, cancel: &AtomicBool) -> io::Result<()> {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        for ent in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let p = ent.path();
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                emit(sink, id, &Frame::TreeEntry { rel, is_dir: true, size: 0, mtime_ms: 0 })?;
                walk(sink, id, base, &p, cancel)?;
            } else if ft.is_file() {
                let md = ent.metadata().ok();
                let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                let mtime = md.as_ref().and_then(|m| m.modified().ok()).map(systemtime_ms).unwrap_or(0);
                emit(sink, id, &Frame::TreeEntry { rel, is_dir: false, size, mtime_ms: mtime })?;
                let mut f = std::fs::File::open(&p)?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let n = f.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
                }
            }
        }
        Ok(())
    }
    let base = Path::new(root);
    walk(sink, id, base, base, cancel)?;
    emit(sink, id, &Frame::End)
}

/// Receive an entire subtree (mirror of `handle_get_tree`) under `root`.
fn handle_put_tree(
    sink: &Sink,
    id: u64,
    root: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let base = Path::new(root);
    let mut cur: Option<std::fs::File> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::TreeEntry { rel, is_dir, .. }) => {
                cur = None;
                let dst = base.join(&rel);
                if is_dir {
                    std::fs::create_dir_all(&dst)?;
                } else {
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    cur = Some(std::fs::File::create(&dst)?);
                }
            }
            Ok(Frame::Data(d)) => {
                if let Some(f) = cur.as_mut() {
                    f.write_all(&d)?;
                }
            }
            Ok(Frame::End) => break,
            Ok(_) => {}
            Err(_) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "put-tree aborted")),
        }
    }
    emit(sink, id, &Frame::Ok)
}

fn glob_match(pat: &str, s: &str) -> bool {
    // Minimal `*`/`?` glob (case-insensitive), iterative with backtracking.
    let (p, t): (Vec<char>, Vec<char>) =
        (pat.to_lowercase().chars().collect(), s.to_lowercase().chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches_spec(name: &str, is_dir: bool, size: u64, spec: &SearchSpec) -> bool {
    if is_dir && !spec.want_dirs {
        return false;
    }
    if !is_dir {
        if size < spec.min_size {
            return false;
        }
        if spec.max_size != 0 && size > spec.max_size {
            return false;
        }
    }
    if spec.query.is_empty() {
        return true;
    }
    if spec.glob {
        glob_match(&spec.query, name)
    } else {
        name.to_lowercase().contains(&spec.query.to_lowercase())
    }
}

/// Recursive server-side search → stream `Match` per hit, then `End`.
fn handle_search(sink: &Sink, id: u64, root: &str, spec: &SearchSpec, cancel: &AtomicBool) -> io::Result<()> {
    let base = Path::new(root);
    let mut count = 0u64;
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let p = ent.path();
            let nm = ent.file_name().to_string_lossy().into_owned();
            let md = ent.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = md.as_ref().and_then(|m| m.modified().ok()).map(systemtime_ms).unwrap_or(0);
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                stack.push(p.clone());
            }
            if matches_spec(&nm, ft.is_dir(), size, spec) {
                let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                emit(sink, id, &Frame::Match { rel, is_dir: ft.is_dir(), size, mtime_ms: mtime })?;
                count += 1;
                if spec.max_results != 0 && count >= spec.max_results {
                    return emit(sink, id, &Frame::End);
                }
            }
        }
    }
    emit(sink, id, &Frame::End)
}

/// Walk `root` emitting size+mtime (and optionally md5) per file → `HashEntry`*
/// then `End`. The sync signature in one server-side pass.
fn handle_walk_hashed(sink: &Sink, id: u64, root: &str, want_hash: bool, cancel: &AtomicBool) -> io::Result<()> {
    let base = Path::new(root);
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let p = ent.path();
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                emit(sink, id, &Frame::HashEntry { rel, is_dir: true, size: 0, mtime_ms: 0, md5: None })?;
                stack.push(p.clone());
            } else if ft.is_file() {
                let md = ent.metadata().ok();
                let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                let mtime = md.as_ref().and_then(|m| m.modified().ok()).map(systemtime_ms).unwrap_or(0);
                let md5 = if want_hash { md5_file(&p).ok() } else { None };
                emit(sink, id, &Frame::HashEntry { rel, is_dir: false, size, mtime_ms: mtime, md5 })?;
            }
        }
    }
    emit(sink, id, &Frame::End)
}

// ── serve loop (the agent's main) ────────────────────────────────────────────

/// Drive the agent: read framed requests from `r`, dispatch each on its own
/// thread (so a long transfer never blocks a quick listing), and route inbound
/// `Data`/`End`/`TreeEntry` chunks of upload streams to the right handler by id.
/// Writes are serialised through a shared mutexed sink.
pub fn serve(mut r: impl Read, w: impl Write + Send + 'static) -> io::Result<()> {
    let sink: Sink = Arc::new(Mutex::new(Box::new(w)));
    // Active upload streams (Write/PutTree): req_id → channel feeding the handler.
    let inbound: Arc<Mutex<HashMap<u64, Sender<Frame>>>> = Arc::new(Mutex::new(HashMap::new()));
    // Cancellation flags by req_id.
    let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));

    while let Some((id, frame)) = read_frame(&mut r)? {
        match frame {
            // Inbound stream chunk → route to the active handler for this id.
            Frame::Data(_) | Frame::TreeEntry { .. } | Frame::End => {
                let tx = inbound.lock().unwrap().get(&id).cloned();
                if let Some(tx) = tx {
                    let is_end = matches!(frame, Frame::End);
                    let _ = tx.send(frame);
                    if is_end {
                        inbound.lock().unwrap().remove(&id);
                    }
                }
            }
            Frame::Cancel => {
                if let Some(f) = cancels.lock().unwrap().get(&id) {
                    f.store(true, Ordering::Relaxed);
                }
            }
            req => {
                let cancel = Arc::new(AtomicBool::new(false));
                cancels.lock().unwrap().insert(id, cancel.clone());
                // Upload-style requests need an inbound channel for their data.
                let rx = match &req {
                    Frame::Write(_) | Frame::PutTree(_) => {
                        let (tx, rx) = channel();
                        inbound.lock().unwrap().insert(id, tx);
                        Some(rx)
                    }
                    _ => None,
                };
                let sink2 = sink.clone();
                let cancels2 = cancels.clone();
                let inbound2 = inbound.clone();
                std::thread::spawn(move || {
                    let res = dispatch(&sink2, id, req, rx.as_ref(), &cancel);
                    if let Err(e) = res {
                        let _ = emit(&sink2, id, &Frame::Err(e.to_string()));
                    }
                    cancels2.lock().unwrap().remove(&id);
                    inbound2.lock().unwrap().remove(&id);
                });
            }
        }
    }
    Ok(())
}

/// Run one request to completion, emitting its response frame(s).
fn dispatch(
    sink: &Sink,
    id: u64,
    req: Frame,
    inbound: Option<&Receiver<Frame>>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    match req {
        Frame::Hello { .. } => emit(
            sink,
            id,
            &Frame::HelloOk { proto: PROTO_VERSION, version: env!("CARGO_PKG_VERSION").to_string() },
        ),
        Frame::ListDir(p) => match list_local(&p) {
            Ok(v) => emit(sink, id, &Frame::Dir(v)),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Stat(p) => match stat_local(&p) {
            Ok(m) => emit(sink, id, &Frame::Meta(m)),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::WalkTree(p) => handle_walk_tree(sink, id, &p, cancel),
        Frame::Read { path, offset, len } => handle_read(sink, id, &path, offset, len, cancel),
        Frame::Write(p) => match inbound {
            Some(rx) => handle_write(sink, id, &p, rx, cancel),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::Copy { src, dst } => match std::fs::copy(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Rename { src, dst } => match std::fs::rename(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Remove { path, recursive } => match remove_path(&path, recursive) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Mkdir(p) => match std::fs::create_dir_all(&p) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::GetTree(root) => handle_get_tree(sink, id, &root, cancel),
        Frame::PutTree(root) => match inbound {
            Some(rx) => handle_put_tree(sink, id, &root, rx, cancel),
            None => emit(sink, id, &Frame::Err("put-tree: no inbound channel".into())),
        },
        Frame::Search { root, spec } => handle_search(sink, id, &root, &spec, cancel),
        Frame::WalkHashed { root, want_hash } => handle_walk_hashed(sink, id, &root, want_hash, cancel),
        other => emit(sink, id, &Frame::Err(format!("unsupported request: {other:?}"))),
    }
}

// ── md5 (pure-Rust, RFC 1321) — for WalkHashed checksum mode ──────────────────
// The agent crate links no hashing dependency; this small implementation keeps
// it self-contained and musl-clean.

fn md5_file(path: &Path) -> io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut ctx = Md5::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx.finish_hex())
}

struct Md5 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    len: u64,
    buf: [u8; 64],
    buf_len: usize,
}
impl Md5 {
    fn new() -> Self {
        Md5 { a: 0x67452301, b: 0xefcdab89, c: 0x98badcfe, d: 0x10325476, len: 0, buf: [0; 64], buf_len: 0 }
    }
    fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            let take = need.min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.process(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.process(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }
    fn finish_hex(mut self) -> String {
        let bit_len = self.len.wrapping_mul(8);
        let mut pad = [0u8; 72];
        pad[0] = 0x80;
        let padlen = if self.buf_len < 56 { 56 - self.buf_len } else { 120 - self.buf_len };
        self.update(&pad[..padlen]);
        let lb = bit_len.to_le_bytes();
        self.update(&lb);
        let mut out = String::with_capacity(32);
        for v in [self.a, self.b, self.c, self.d] {
            for byte in v.to_le_bytes() {
                out.push_str(&format!("{:02x}", byte));
            }
        }
        out
    }
    fn process(&mut self, block: &[u8; 64]) {
        const S: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
            14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
            21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        const K: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
            0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
            0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
            0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
            0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
            0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
            0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
        ];
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (self.a, self.b, self.c, self.d);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        self.a = self.a.wrapping_add(a);
        self.b = self.b.wrapping_add(b);
        self.c = self.c.wrapping_add(c);
        self.d = self.d.wrapping_add(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let tree = WireNode {
            name: "r".into(),
            size: 500,
            is_dir: true,
            children: vec![WireNode { name: "a".into(), size: 100, is_dir: false, children: vec![] }],
        };
        let frames = [
            Frame::Hello { proto: 7 },
            Frame::HelloOk { proto: 2, version: "0.1".into() },
            Frame::ListDir("/a/b".into()),
            Frame::Dir(vec![WireMeta { name: "f".into(), is_dir: false, is_symlink: false, size: 9, mtime_ms: 1 }]),
            Frame::Stat("/x".into()),
            Frame::Meta(WireMeta { name: "d".into(), is_dir: true, is_symlink: false, size: 0, mtime_ms: 0 }),
            Frame::WalkTree("/".into()),
            Frame::Tree(tree),
            Frame::Read { path: "/f".into(), offset: 10, len: 0 },
            Frame::Write("/f".into()),
            Frame::Data(vec![1, 2, 3, 4]),
            Frame::Copy { src: "/a".into(), dst: "/b".into() },
            Frame::Rename { src: "/a".into(), dst: "/b".into() },
            Frame::Remove { path: "/x".into(), recursive: true },
            Frame::Mkdir("/d".into()),
            Frame::GetTree("/r".into()),
            Frame::PutTree("/r".into()),
            Frame::TreeEntry { rel: "a/b".into(), is_dir: false, size: 7, mtime_ms: 3 },
            Frame::Search {
                root: "/r".into(),
                spec: SearchSpec { query: "x".into(), glob: true, min_size: 1, max_size: 9, max_results: 5, want_dirs: true },
            },
            Frame::Match { rel: "a".into(), is_dir: false, size: 1, mtime_ms: 0 },
            Frame::WalkHashed { root: "/r".into(), want_hash: true },
            Frame::HashEntry { rel: "a".into(), is_dir: false, size: 1, mtime_ms: 0, md5: Some("abc".into()) },
            Frame::Progress { done: 3, total: 9 },
            Frame::Ok,
            Frame::End,
            Frame::Err("nope".into()),
            Frame::Cancel,
        ];
        for f in frames {
            let (id, got) = Frame::decode(&f.encode(42)).unwrap();
            assert_eq!(id, 42);
            assert_eq!(got, f);
        }
    }

    #[test]
    fn glob_matches() {
        assert!(glob_match("*.txt", "a.txt"));
        assert!(glob_match("foo?", "foob"));
        assert!(!glob_match("*.txt", "a.bin"));
        assert!(glob_match("*report*", "Q3_Report_final"));
    }

    #[test]
    fn md5_known_vectors() {
        let mut m = Md5::new();
        m.update(b"");
        assert_eq!(m.finish_hex(), "d41d8cd98f00b204e9800998ecf8427e");
        let mut m = Md5::new();
        m.update(b"abc");
        assert_eq!(m.finish_hex(), "900150983cd24fb0d6963f7d28e17f72");
        let mut m = Md5::new();
        m.update(b"The quick brown fox jumps over the lazy dog");
        assert_eq!(m.finish_hex(), "9e107d9d372bb6826bd81d3542a419d6");
    }
}
