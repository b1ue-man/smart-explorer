use std::io::{self, Read, Write};

use super::types::{Frame, SearchSpec, WireMeta, WireNode};

/// Reject absurd frame lengths from a corrupt/hostile stream before allocating.
const MAX_FRAME: usize = 1 << 31; // 2 GiB

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
        Ok(if self.bool()? {
            Some(self.string()?)
        } else {
            None
        })
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
    Ok(WireNode {
        name,
        size,
        is_dir,
        children,
    })
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
        }
        b
    }

    pub fn decode(body: &[u8]) -> io::Result<(u64, Frame)> {
        let mut r = Reader::new(body);
        let req_id = r.u64()?;
        let frame = match r.u8()? {
            1 => Frame::Hello { proto: r.u32()? },
            2 => Frame::HelloOk {
                proto: r.u32()?,
                version: r.string()?,
            },
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
            9 => Frame::Read {
                path: r.string()?,
                offset: r.u64()?,
                len: r.u64()?,
            },
            10 => Frame::Write(r.string()?),
            11 => Frame::Data(r.bytes()?),
            12 => Frame::Copy {
                src: r.string()?,
                dst: r.string()?,
            },
            13 => Frame::Rename {
                src: r.string()?,
                dst: r.string()?,
            },
            14 => Frame::Remove {
                path: r.string()?,
                recursive: r.bool()?,
            },
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
            21 => Frame::WalkHashed {
                root: r.string()?,
                want_hash: r.bool()?,
            },
            22 => Frame::HashEntry {
                rel: r.string()?,
                is_dir: r.bool()?,
                size: r.u64()?,
                mtime_ms: r.i64()?,
                md5: r.opt_str()?,
            },
            23 => Frame::Progress {
                done: r.u64()?,
                total: r.u64()?,
            },
            24 => Frame::Ok,
            25 => Frame::End,
            26 => Frame::Err(r.string()?),
            27 => Frame::Cancel,
            t => return Err(bad(&format!("unknown frame tag {t}"))),
        };
        Ok((req_id, frame))
    }
}

pub fn write_frame(w: &mut impl Write, req_id: u64, frame: &Frame) -> io::Result<()> {
    let body = frame.encode(req_id);
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

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
