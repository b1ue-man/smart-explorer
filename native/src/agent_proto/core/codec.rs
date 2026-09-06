use std::io;

use super::node_codec::decode_node;
use super::relative_path::ValidatedRelativePath;
use super::types::{Frame, SearchSpec, WireMeta, CHUNK};

pub(super) const MAX_FRAME: usize = 64 * 1024 * 1024;
// Empty-name WireMeta: u32 name length, two flags, u64 size, i64 mtime,
// and one optional-md5 flag. Variable string bytes can only increase this.
pub(super) const MIN_WIRE_META_BYTES: usize = 23;

#[path = "frame_encode.rs"]
mod frame_encode;
#[path = "frame_io.rs"]
mod frame_io;
pub use frame_io::{read_frame, write_frame};

pub(super) fn validate_frame_len(len: usize) -> io::Result<()> {
    (len <= MAX_FRAME)
        .then_some(())
        .ok_or_else(|| bad("frame too large"))
}

pub(super) struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, i: 0 }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(bad("truncated frame"));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(super) fn u32(&mut self) -> io::Result<u32> {
        let bytes = self
            .take(4)?
            .try_into()
            .map_err(|_| bad("invalid u32 field length"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) fn u64(&mut self) -> io::Result<u64> {
        let bytes = self
            .take(8)?
            .try_into()
            .map_err(|_| bad("invalid u64 field length"))?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn i64(&mut self) -> io::Result<i64> {
        let bytes = self
            .take(8)?
            .try_into()
            .map_err(|_| bad("invalid i64 field length"))?;
        Ok(i64::from_le_bytes(bytes))
    }

    pub(super) fn bool(&mut self) -> io::Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub(super) fn string(&mut self) -> io::Result<String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| bad("invalid utf8"))
    }

    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    fn bounded_bytes(&mut self, maximum: usize) -> io::Result<Vec<u8>> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(bad("data frame exceeds the protocol chunk size"));
        }
        Ok(self.take(length)?.to_vec())
    }

    fn remaining(&self) -> usize {
        self.b.len() - self.i
    }

    fn is_finished(&self) -> bool {
        self.i == self.b.len()
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

fn get_meta(r: &mut Reader) -> io::Result<WireMeta> {
    Ok(WireMeta {
        name: r.string()?,
        is_dir: r.bool()?,
        is_symlink: r.bool()?,
        size: r.u64()?,
        mtime_ms: r.i64()?,
        content_md5: r.opt_str()?,
    })
}

impl Frame {
    pub fn decode(body: &[u8]) -> io::Result<(u64, Frame)> {
        validate_frame_len(body.len())?;
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
                if n > r.remaining() / MIN_WIRE_META_BYTES {
                    return Err(bad(
                        "directory entry count exceeds the remaining frame bytes",
                    ));
                }
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(get_meta(&mut r)?);
                }
                Frame::Dir(v)
            }
            5 => Frame::Stat(r.string()?),
            6 => Frame::Meta(get_meta(&mut r)?),
            7 => Frame::WalkTree(r.string()?),
            8 => Frame::Tree(decode_node(&mut r)?),
            9 => Frame::Read {
                path: r.string()?,
                offset: r.u64()?,
                len: r.u64()?,
            },
            10 => Frame::Write(r.string()?),
            11 => Frame::Data(r.bounded_bytes(CHUNK)?),
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
            18 => {
                let rel = ValidatedRelativePath::parse(&r.string()?)?;
                Frame::TreeEntry {
                    rel: rel.as_str().to_string(),
                    is_dir: r.bool()?,
                    size: r.u64()?,
                    mtime_ms: r.i64()?,
                }
            }
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
            28 => Frame::TryExists(r.string()?),
            29 => Frame::Exists(r.bool()?),
            30 => Frame::RenameNoReplace {
                src: r.string()?,
                dst: r.string()?,
            },
            31 => Frame::Promote {
                staged: r.string()?,
                destination: r.string()?,
            },
            32 => Frame::PromoteNoReplace {
                staged: r.string()?,
                destination: r.string()?,
            },
            33 => Frame::WriteNew(r.string()?),
            t => return Err(bad(&format!("unknown frame tag {t}"))),
        };
        if !r.is_finished() {
            return Err(bad("trailing bytes after frame payload"));
        }
        Ok((req_id, frame))
    }
}
