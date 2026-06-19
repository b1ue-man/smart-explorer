use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::core_oslocked::{is_pseudo_dir, systemtime_ms};
use super::session::{emit, Sink};
use super::{Frame, CHUNK};

/// Walk `root` emitting size+mtime (and optionally md5) per file.
pub(crate) fn handle_walk_hashed(
    sink: &Sink,
    id: u64,
    root: &str,
    want_hash: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
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
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if ft.is_dir() {
                if is_pseudo_dir(&p.to_string_lossy()) {
                    continue;
                }
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel,
                        is_dir: true,
                        size: 0,
                        mtime_ms: 0,
                        md5: None,
                    },
                )?;
                stack.push(p.clone());
            } else if ft.is_file() {
                let md = ent.metadata().ok();
                let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                let mtime = md
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(systemtime_ms)
                    .unwrap_or(0);
                let md5 = if want_hash { md5_file(&p).ok() } else { None };
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel,
                        is_dir: false,
                        size,
                        mtime_ms: mtime,
                        md5,
                    },
                )?;
            }
        }
    }
    emit(sink, id, &Frame::End)
}

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
        Md5 {
            a: 0x67452301,
            b: 0xefcdab89,
            c: 0x98badcfe,
            d: 0x10325476,
            len: 0,
            buf: [0; 64],
            buf_len: 0,
        }
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
        let padlen = if self.buf_len < 56 {
            56 - self.buf_len
        } else {
            120 - self.buf_len
        };
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
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20,
            5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
            6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        const K: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
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
    use super::Md5;

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
