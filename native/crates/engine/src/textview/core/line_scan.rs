//! Streaming line splitter over a [`TextSource`]: reads fixed chunks, finds
//! CR LF, LF and CR endings in code units (bytes for UTF-8/Windows-1252,
//! 16-bit units for UTF-16) and reports every line with its byte range. A
//! file with N line endings has N + 1 lines (the last one may be empty), as
//! Notepad++ and VS Code count them.
use std::io;

use serde::{Deserialize, Serialize};

use super::encoding::TextEncoding;
use super::source::{read_full_at, TextSource};

const LF: u16 = 0x0A;
const CR: u16 = 0x0D;

/// How a line ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// Windows: CR LF.
    CrLf,
    /// Unix: LF.
    Lf,
    /// Classic Mac OS: CR alone.
    Cr,
}

impl LineEnding {
    /// Name the surfaces show in the status line.
    pub fn label(self) -> &'static str {
        match self {
            Self::CrLf => "CRLF",
            Self::Lf => "LF",
            Self::Cr => "CR",
        }
    }
}

/// Code unit layout of an encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    width: usize,
    big_endian: bool,
}

impl Layout {
    pub(crate) fn of(encoding: TextEncoding) -> Self {
        Self {
            width: encoding.unit_width(),
            big_endian: encoding.big_endian(),
        }
    }
}

/// Byte range of one line: content `[start, end)`, next line at `next`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineSpan {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) next: u64,
    /// `None` for the last line of the file.
    pub(crate) ending: Option<LineEnding>,
}

/// Receives the chunks and lines of a walk; returning `false` stops it.
pub(crate) trait LineVisitor {
    /// Every chunk, before its lines are reported.
    fn chunk(&mut self, _offset: u64, _bytes: &[u8]) -> bool {
        true
    }

    /// One line; `bytes` holds its content when the line lies inside the
    /// current chunk (otherwise the caller reads the range itself).
    fn line(&mut self, span: LineSpan, bytes: Option<&[u8]>) -> bool;
}

/// How a walk ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkEnd {
    /// The last line of the source was reported.
    Finished,
    /// The visitor stopped the walk.
    Stopped,
}

/// Walks the lines from `start` (the first byte of a line, aligned to the
/// code unit) to the end of the source.
pub(crate) fn walk_lines(
    source: &dyn TextSource,
    layout: Layout,
    start: u64,
    chunk_size: usize,
    visitor: &mut dyn LineVisitor,
) -> io::Result<WalkEnd> {
    let w = layout.width;
    let chunk_size = chunk_size.max(64) & !1;
    let mut buf = vec![0u8; chunk_size];
    let mut chunk_start = start;
    let mut line_start = start;
    let mut pending_cr: Option<u64> = None;
    loop {
        let n = read_full_at(source, chunk_start, &mut buf)?;
        let chunk = &buf[..n];
        let last = n < chunk_size;
        if !visitor.chunk(chunk_start, chunk) {
            return Ok(WalkEnd::Stopped);
        }
        let mut emitter = Emitter {
            visitor: &mut *visitor,
            chunk_start,
            chunk,
        };
        let mut i = 0usize;
        if let Some(cr) = pending_cr.take() {
            let crlf = unit_at(chunk, 0, layout) == Some(LF);
            let (next, ending) = if crlf {
                (cr + 2 * w as u64, LineEnding::CrLf)
            } else {
                (cr + w as u64, LineEnding::Cr)
            };
            if !emitter.emit(line_start, cr, next, Some(ending)) {
                return Ok(WalkEnd::Stopped);
            }
            line_start = next;
            i = (next - chunk_start) as usize;
        }
        while let Some(h) = find_terminator(chunk, i, layout) {
            let abs = chunk_start + h as u64;
            let after = h + w;
            let (next, ending) = if unit_at(chunk, h, layout) == Some(LF) {
                (abs + w as u64, LineEnding::Lf)
            } else if after + w <= chunk.len() {
                if unit_at(chunk, after, layout) == Some(LF) {
                    (abs + 2 * w as u64, LineEnding::CrLf)
                } else {
                    (abs + w as u64, LineEnding::Cr)
                }
            } else if last {
                (abs + w as u64, LineEnding::Cr)
            } else {
                // A CR at the end of a full chunk: the next chunk decides.
                pending_cr = Some(abs);
                break;
            };
            if !emitter.emit(line_start, abs, next, Some(ending)) {
                return Ok(WalkEnd::Stopped);
            }
            line_start = next;
            i = (next - chunk_start) as usize;
        }
        if last {
            let end = chunk_start + n as u64;
            emitter.emit(line_start, end, end, None);
            return Ok(WalkEnd::Finished);
        }
        chunk_start += n as u64;
    }
}

struct Emitter<'a, 'b, 'v> {
    visitor: &'a mut (dyn LineVisitor + 'v),
    chunk_start: u64,
    chunk: &'b [u8],
}

impl Emitter<'_, '_, '_> {
    fn emit(&mut self, start: u64, end: u64, next: u64, ending: Option<LineEnding>) -> bool {
        let chunk_end = self.chunk_start + self.chunk.len() as u64;
        let bytes = (start >= self.chunk_start && end <= chunk_end).then(|| {
            &self.chunk[(start - self.chunk_start) as usize..(end - self.chunk_start) as usize]
        });
        self.visitor.line(
            LineSpan {
                start,
                end,
                next,
                ending,
            },
            bytes,
        )
    }
}

/// Code unit at byte index `i` (aligned), if the chunk holds all of it.
fn unit_at(chunk: &[u8], i: usize, layout: Layout) -> Option<u16> {
    if layout.width == 1 {
        return chunk.get(i).map(|&b| u16::from(b));
    }
    let pair = chunk.get(i..i + 2)?;
    Some(if layout.big_endian {
        u16::from_be_bytes([pair[0], pair[1]])
    } else {
        u16::from_le_bytes([pair[0], pair[1]])
    })
}

/// Byte index of the next CR or LF code unit at or after `from` (aligned).
fn find_terminator(chunk: &[u8], from: usize, layout: Layout) -> Option<usize> {
    if from >= chunk.len() {
        return None;
    }
    if layout.width == 1 {
        return memchr::memchr2(b'\n', b'\r', &chunk[from..]).map(|p| p + from);
    }
    let mut pos = from;
    while let Some(p) = memchr::memchr2(b'\n', b'\r', &chunk[pos..]) {
        let h = pos + p;
        let unit_start = if layout.big_endian {
            // Low byte second: the unit starts one byte earlier.
            if h % 2 == 1 && chunk[h - 1] == 0 {
                Some(h - 1)
            } else {
                None
            }
        } else if h % 2 == 0 && chunk.get(h + 1) == Some(&0) {
            Some(h)
        } else {
            None
        };
        if let Some(unit_start) = unit_start.filter(|&s| s >= from) {
            let unit = unit_at(chunk, unit_start, layout);
            if unit == Some(LF) || unit == Some(CR) {
                return Some(unit_start);
            }
        }
        pos = h + 1;
        if pos >= chunk.len() {
            break;
        }
    }
    None
}
