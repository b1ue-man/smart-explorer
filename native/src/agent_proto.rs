//! Shared, dependency-free (std + rayon only) core for the SSH remote agent —
//! the wire PROTOCOL and the LOCAL filesystem operations it runs server-side.
//!
//! Included by BOTH the app (the `AgentBackend` transport) and the `se-agent`
//! binary, so there is exactly one definition of the frames and the walk, and
//! the agent binary pulls in nothing else (no vfs/analytics/GUI/TLS) → it stays
//! tiny and cross-compiles cleanly to static musl. See `docs/SSH_AGENT_PLAN.md`.
//!
//! Framing: each message is `u32 LE length` followed by that many body bytes.
//! The body is a compact hand-rolled binary encoding (no serde_json — a
//! million-node `WalkTree` response must not pay JSON's size/parse cost).

use rayon::prelude::*;
use std::io::{self, Read, Write};
use std::path::Path;

/// Bumped whenever the wire format changes; the client re-uploads the agent on a
/// mismatch (handshake in `Req::Hello` / `Resp::Hello`).
pub const PROTO_VERSION: u32 = 1;

/// Reject absurd frame lengths from a corrupt/hostile stream before allocating.
const MAX_FRAME: usize = 1 << 31; // 2 GiB

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

/// Client → agent.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    Hello { proto: u32 },
    ListDir(String),
    Stat(String),
    WalkTree(String),
}

/// Agent → client.
#[derive(Clone, Debug, PartialEq)]
pub enum Resp {
    Hello { proto: u32, version: String },
    Dir(Vec<WireMeta>),
    Meta(WireMeta),
    Tree(WireNode),
    Err(String),
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

impl Req {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Req::Hello { proto } => {
                b.push(1);
                put_u32(&mut b, *proto);
            }
            Req::ListDir(p) => {
                b.push(2);
                put_str(&mut b, p);
            }
            Req::Stat(p) => {
                b.push(3);
                put_str(&mut b, p);
            }
            Req::WalkTree(p) => {
                b.push(4);
                put_str(&mut b, p);
            }
        }
        b
    }
    pub fn decode(body: &[u8]) -> io::Result<Req> {
        let mut r = Reader::new(body);
        Ok(match r.u8()? {
            1 => Req::Hello { proto: r.u32()? },
            2 => Req::ListDir(r.string()?),
            3 => Req::Stat(r.string()?),
            4 => Req::WalkTree(r.string()?),
            t => return Err(bad(&format!("unknown req tag {t}"))),
        })
    }
}

impl Resp {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        match self {
            Resp::Hello { proto, version } => {
                b.push(1);
                put_u32(&mut b, *proto);
                put_str(&mut b, version);
            }
            Resp::Dir(v) => {
                b.push(2);
                put_u32(&mut b, v.len() as u32);
                for m in v {
                    put_meta(&mut b, m);
                }
            }
            Resp::Meta(m) => {
                b.push(3);
                put_meta(&mut b, m);
            }
            Resp::Tree(n) => {
                b.push(4);
                put_node(&mut b, n);
            }
            Resp::Err(e) => {
                b.push(5);
                put_str(&mut b, e);
            }
        }
        b
    }
    pub fn decode(body: &[u8]) -> io::Result<Resp> {
        let mut r = Reader::new(body);
        Ok(match r.u8()? {
            1 => Resp::Hello { proto: r.u32()?, version: r.string()? },
            2 => {
                let n = r.u32()? as usize;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(get_meta(&mut r)?);
                }
                Resp::Dir(v)
            }
            3 => Resp::Meta(get_meta(&mut r)?),
            4 => Resp::Tree(get_node(&mut r)?),
            5 => Resp::Err(r.string()?),
            t => return Err(bad(&format!("unknown resp tag {t}"))),
        })
    }
}

// ── framing ──────────────────────────────────────────────────────────────────

/// Write a length-prefixed frame and flush.
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Read one frame. `Ok(None)` = clean EOF before any byte of the next frame.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
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
    Ok(Some(body))
}

// ── local filesystem operations (run server-side by the agent) ───────────────

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
                subdirs.push((ent.path(), nm));
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

// ── serve loop (the agent's main) ────────────────────────────────────────────

/// Dispatch one request to a response.
pub fn handle(req: Req) -> Resp {
    match req {
        Req::Hello { .. } => Resp::Hello {
            proto: PROTO_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        Req::ListDir(p) => match list_local(&p) {
            Ok(v) => Resp::Dir(v),
            Err(e) => Resp::Err(e.to_string()),
        },
        Req::Stat(p) => match stat_local(&p) {
            Ok(m) => Resp::Meta(m),
            Err(e) => Resp::Err(e.to_string()),
        },
        Req::WalkTree(p) => Resp::Tree(walk_local(Path::new(&p))),
    }
}

/// Read framed requests from `r`, write framed responses to `w`, until EOF.
pub fn serve(mut r: impl Read, mut w: impl Write) -> io::Result<()> {
    while let Some(frame) = read_frame(&mut r)? {
        let resp = match Req::decode(&frame) {
            Ok(req) => handle(req),
            Err(e) => Resp::Err(format!("decode: {e}")),
        };
        write_frame(&mut w, &resp.encode())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_roundtrip() {
        for r in [
            Req::Hello { proto: 7 },
            Req::ListDir("/a/b".into()),
            Req::Stat("/x".into()),
            Req::WalkTree("/".into()),
        ] {
            assert_eq!(Req::decode(&r.encode()).unwrap(), r);
        }
    }

    #[test]
    fn resp_roundtrip() {
        let tree = WireNode {
            name: "r".into(),
            size: 500,
            is_dir: true,
            children: vec![
                WireNode {
                    name: "sub".into(),
                    size: 400,
                    is_dir: true,
                    children: vec![WireNode { name: "b".into(), size: 400, is_dir: false, children: vec![] }],
                },
                WireNode { name: "a".into(), size: 100, is_dir: false, children: vec![] },
            ],
        };
        for r in [
            Resp::Hello { proto: 1, version: "0.1".into() },
            Resp::Dir(vec![WireMeta { name: "f".into(), is_dir: false, is_symlink: false, size: 9, mtime_ms: 123 }]),
            Resp::Meta(WireMeta { name: "d".into(), is_dir: true, is_symlink: false, size: 0, mtime_ms: 0 }),
            Resp::Tree(tree.clone()),
            Resp::Err("nope".into()),
        ] {
            assert_eq!(Resp::decode(&r.encode()).unwrap(), r);
        }
    }

    #[test]
    fn framed_serve_roundtrip() {
        // Drive the serve() dispatch over an in-memory pipe (no process/SSH).
        let base = std::env::temp_dir().join(format!("se_agent_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("sub/b.bin"), vec![0u8; 400]).unwrap();
        let root = base.to_string_lossy().to_string();

        // Build a request stream.
        let mut input = Vec::new();
        write_frame(&mut input, &Req::Hello { proto: PROTO_VERSION }.encode()).unwrap();
        write_frame(&mut input, &Req::WalkTree(root.clone()).encode()).unwrap();
        write_frame(&mut input, &Req::ListDir(root.clone()).encode()).unwrap();

        let mut output = Vec::new();
        serve(&input[..], &mut output).unwrap();

        // Decode the three responses back.
        let mut rd = &output[..];
        let hello = Resp::decode(&read_frame(&mut rd).unwrap().unwrap()).unwrap();
        assert!(matches!(hello, Resp::Hello { proto, .. } if proto == PROTO_VERSION));
        let tree = Resp::decode(&read_frame(&mut rd).unwrap().unwrap()).unwrap();
        match tree {
            Resp::Tree(n) => {
                assert_eq!(n.size, 500);
                let sub = n.children.iter().find(|c| c.name == "sub").unwrap();
                assert_eq!(sub.size, 400);
            }
            _ => panic!("expected Tree"),
        }
        let dir = Resp::decode(&read_frame(&mut rd).unwrap().unwrap()).unwrap();
        match dir {
            Resp::Dir(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected Dir"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
