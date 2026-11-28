//! A text document of any size over a [`TextSource`]: the encoding decided
//! from the first 8 KiB, a streaming line index that keeps only the byte
//! offset of every [`LINE_STRIDE`]th line, and window reads of a few lines
//! that never hold the whole file in memory. The index is built by
//! [`TextDocument::build_index`] (usually on a background thread) and is
//! usable while it grows: reads walk from the nearest known offset.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use serde::Serialize;

use super::encoding::{detect_encoding, TextEncoding, Utf8Validator, SNIFF_LEN};
use super::error::{TextViewError, TextViewResult};
use super::line_scan::{walk_lines, Layout, LineEnding, LineSpan, LineVisitor, WalkEnd};
use super::source::{read_full_at, BytesSource, TextSource};

/// Every this many lines the index keeps the byte offset of a line start.
pub const LINE_STRIDE: u64 = 1024;
/// A line longer than this many bytes is returned cut (`TextLine::truncated`).
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const INDEX_CHUNK: usize = 1024 * 1024;
const WINDOW_CHUNK: usize = 64 * 1024;

/// How many lines ended with which line ending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LineEndingCounts {
    pub crlf: u64,
    pub lf: u64,
    pub cr: u64,
}

impl LineEndingCounts {
    fn add(&mut self, ending: LineEnding) {
        match ending {
            LineEnding::CrLf => self.crlf += 1,
            LineEnding::Lf => self.lf += 1,
            LineEnding::Cr => self.cr += 1,
        }
    }

    /// The most frequent ending; `None` for a file without line endings.
    pub fn dominant(&self) -> Option<LineEnding> {
        let all = [
            (self.crlf, LineEnding::CrLf),
            (self.lf, LineEnding::Lf),
            (self.cr, LineEnding::Cr),
        ];
        all.iter()
            .filter(|(n, _)| *n > 0)
            .max_by_key(|(n, _)| *n)
            .map(|(_, ending)| *ending)
    }

    /// More than one kind of line ending occurs.
    pub fn is_mixed(&self) -> bool {
        [self.crlf, self.lf, self.cr]
            .iter()
            .filter(|n| **n > 0)
            .count()
            > 1
    }
}

/// State of the line index, for progress display and the status line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IndexStatus {
    /// Lines found so far; the final line count once `complete`.
    pub lines: u64,
    pub complete: bool,
    pub bytes_done: u64,
    pub total_bytes: u64,
    /// Line endings counted so far.
    pub endings: LineEndingCounts,
    /// Current encoding; a file without BOM switches from UTF-8 to
    /// Windows-1252 when the index meets bytes that are not UTF-8.
    pub encoding: TextEncoding,
}

/// One line of a window read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TextLine {
    /// Zero-based line number.
    pub number: u64,
    pub text: String,
    /// `None` for the last line of the file.
    pub ending: Option<LineEnding>,
    /// The line has more than [`MAX_LINE_BYTES`] bytes and was cut.
    pub truncated: bool,
}

/// Anything that can hand out a window of lines (documents, a code block).
pub trait LineProvider {
    fn provide_lines(&self, first: u64, count: usize) -> io::Result<Vec<String>>;
}

impl LineProvider for [String] {
    fn provide_lines(&self, first: u64, count: usize) -> io::Result<Vec<String>> {
        let start = usize::try_from(first).unwrap_or(usize::MAX).min(self.len());
        let end = start.saturating_add(count).min(self.len());
        Ok(self[start..end].to_vec())
    }
}

#[derive(Debug)]
struct IndexState {
    checkpoints: Vec<u64>,
    lines: u64,
    complete: bool,
    bytes_done: u64,
    endings: LineEndingCounts,
}

impl IndexState {
    fn fresh(start: u64) -> Self {
        Self {
            checkpoints: vec![start],
            lines: 0,
            complete: false,
            bytes_done: start,
            endings: LineEndingCounts::default(),
        }
    }
}

/// An open text document.
pub struct TextDocument {
    source: Arc<dyn TextSource>,
    len: u64,
    encoding: RwLock<TextEncoding>,
    index: RwLock<IndexState>,
    build: Mutex<()>,
}

impl std::fmt::Debug for TextDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextDocument")
            .field("len", &self.len)
            .field("status", &self.status())
            .finish()
    }
}

impl TextDocument {
    /// Opens a document: reads the first 8 KiB, decides the encoding and
    /// refuses binary data ([`TextViewError::NotText`]).
    pub fn open(source: Arc<dyn TextSource>) -> TextViewResult<Self> {
        let len = source.len();
        let mut prefix = vec![0u8; SNIFF_LEN.min(usize::try_from(len).unwrap_or(SNIFF_LEN))];
        let n = read_full_at(source.as_ref(), 0, &mut prefix)?;
        prefix.truncate(n);
        let encoding = detect_encoding(&prefix, n as u64 >= len)?;
        Ok(Self {
            source,
            len,
            encoding: RwLock::new(encoding),
            index: RwLock::new(IndexState::fresh(encoding.bom_len().min(len))),
            build: Mutex::new(()),
        })
    }

    /// A document over bytes in memory.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> TextViewResult<Self> {
        Self::open(Arc::new(BytesSource::new(bytes)))
    }

    /// Length of the source in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn encoding(&self) -> TextEncoding {
        *self.encoding.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The final line count once the index is complete.
    pub fn line_count(&self) -> Option<u64> {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        index.complete.then_some(index.lines)
    }

    pub fn status(&self) -> IndexStatus {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        IndexStatus {
            lines: index.lines,
            complete: index.complete,
            bytes_done: index.bytes_done,
            total_bytes: self.len,
            endings: index.endings,
            encoding: self.encoding(),
        }
    }

    /// Builds the line index from the start, reporting after every 1 MiB
    /// chunk. Setting `cancel` stops it with [`TextViewError::Cancelled`];
    /// the offsets found so far stay usable, a later call starts over. A
    /// complete index is returned without reading again.
    pub fn build_index(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&IndexStatus),
    ) -> TextViewResult<IndexStatus> {
        let _building = self.build.lock().unwrap_or_else(PoisonError::into_inner);
        if self.line_count().is_some() {
            return Ok(self.status());
        }
        let encoding = self.encoding();
        let start = encoding.bom_len().min(self.len);
        *self.index.write().unwrap_or_else(PoisonError::into_inner) = IndexState::fresh(start);
        let mut builder = IndexBuilder {
            doc: self,
            cancel,
            progress,
            line_no: 0,
            new_checkpoints: Vec::new(),
            endings: LineEndingCounts::default(),
            utf8: (encoding == TextEncoding::Utf8).then(Utf8Validator::default),
        };
        let end = walk_lines(
            self.source.as_ref(),
            Layout::of(encoding),
            start,
            INDEX_CHUNK,
            &mut builder,
        )?;
        if end == WalkEnd::Stopped {
            return Err(TextViewError::Cancelled);
        }
        if builder.utf8.as_ref().is_some_and(|v| !v.finish()) {
            self.set_encoding(TextEncoding::Windows1252);
        }
        builder.publish(self.len, true);
        Ok(self.status())
    }

    fn set_encoding(&self, encoding: TextEncoding) {
        *self
            .encoding
            .write()
            .unwrap_or_else(PoisonError::into_inner) = encoding;
    }

    /// Lines `first .. first + count` as text (fewer at the end of the file).
    pub fn lines(&self, first: u64, count: usize) -> io::Result<Vec<String>> {
        Ok(self
            .read_lines(first, count)?
            .into_iter()
            .map(|line| line.text)
            .collect())
    }

    /// Lines `first .. first + count` with their line ending and cut flag.
    pub fn read_lines(&self, first: u64, count: usize) -> io::Result<Vec<TextLine>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let (start_line, start_offset) = {
            let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
            let k = usize::try_from(first / LINE_STRIDE)
                .unwrap_or(usize::MAX)
                .min(index.checkpoints.len() - 1);
            (k as u64 * LINE_STRIDE, index.checkpoints[k])
        };
        let encoding = self.encoding();
        let mut collector = WindowCollector {
            encoding,
            line_no: start_line,
            first,
            last: first.saturating_add(count as u64),
            lines: Vec::with_capacity(count.min(4096)),
            unread: Vec::new(),
        };
        walk_lines(
            self.source.as_ref(),
            Layout::of(encoding),
            start_offset,
            WINDOW_CHUNK,
            &mut collector,
        )?;
        for (slot, span) in std::mem::take(&mut collector.unread) {
            let len = usize::try_from(span.end - span.start).unwrap_or(usize::MAX);
            let mut bytes = vec![0u8; len.min(MAX_LINE_BYTES)];
            let n = read_full_at(self.source.as_ref(), span.start, &mut bytes)?;
            bytes.truncate(n);
            let (text, truncated) = decode_capped(encoding, &bytes, len);
            collector.lines[slot].text = text;
            collector.lines[slot].truncated = truncated;
        }
        Ok(collector.lines)
    }
}

impl LineProvider for TextDocument {
    fn provide_lines(&self, first: u64, count: usize) -> io::Result<Vec<String>> {
        self.lines(first, count)
    }
}

/// Decodes at most [`MAX_LINE_BYTES`] of a line of `full_len` bytes.
fn decode_capped(encoding: TextEncoding, bytes: &[u8], full_len: usize) -> (String, bool) {
    if full_len <= MAX_LINE_BYTES && bytes.len() == full_len {
        return (encoding.decode(bytes), false);
    }
    let mut cut = bytes.len().min(MAX_LINE_BYTES);
    cut -= cut % encoding.unit_width();
    if matches!(encoding, TextEncoding::Utf8 | TextEncoding::Utf8Bom) {
        cut = utf8_boundary(&bytes[..cut]);
    }
    (encoding.decode(&bytes[..cut]), true)
}

/// Length of `bytes` without a UTF-8 sequence cut at its end.
fn utf8_boundary(bytes: &[u8]) -> usize {
    let len = bytes.len();
    let mut continuation = 0;
    while continuation < 3 && len > continuation && bytes[len - 1 - continuation] & 0xC0 == 0x80 {
        continuation += 1;
    }
    if len == continuation {
        return len;
    }
    let lead = bytes[len - 1 - continuation];
    let needed = match lead {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    };
    if needed > continuation + 1 {
        len - 1 - continuation
    } else {
        len
    }
}

struct IndexBuilder<'a, 'p> {
    doc: &'a TextDocument,
    cancel: &'a AtomicBool,
    progress: &'p mut dyn FnMut(&IndexStatus),
    line_no: u64,
    new_checkpoints: Vec<u64>,
    endings: LineEndingCounts,
    utf8: Option<Utf8Validator>,
}

impl IndexBuilder<'_, '_> {
    fn publish(&mut self, bytes_done: u64, complete: bool) {
        {
            let mut index = self
                .doc
                .index
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            index.checkpoints.append(&mut self.new_checkpoints);
            index.lines = self.line_no;
            index.bytes_done = bytes_done;
            index.endings = self.endings;
            index.complete = complete;
        }
        let status = self.doc.status();
        (self.progress)(&status);
    }
}

impl LineVisitor for IndexBuilder<'_, '_> {
    fn chunk(&mut self, offset: u64, bytes: &[u8]) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(validator) = self.utf8.as_mut() {
            validator.feed(bytes);
            if !validator.is_valid() {
                self.doc.set_encoding(TextEncoding::Windows1252);
                self.utf8 = None;
            }
        }
        if offset > 0 {
            self.publish(offset, false);
        }
        true
    }

    fn line(&mut self, span: LineSpan, _bytes: Option<&[u8]>) -> bool {
        if self.line_no > 0 && self.line_no % LINE_STRIDE == 0 {
            self.new_checkpoints.push(span.start);
        }
        if let Some(ending) = span.ending {
            self.endings.add(ending);
        }
        self.line_no += 1;
        true
    }
}

struct WindowCollector {
    encoding: TextEncoding,
    line_no: u64,
    first: u64,
    last: u64,
    lines: Vec<TextLine>,
    unread: Vec<(usize, LineSpan)>,
}

impl LineVisitor for WindowCollector {
    fn line(&mut self, span: LineSpan, bytes: Option<&[u8]>) -> bool {
        let number = self.line_no;
        self.line_no += 1;
        if number < self.first {
            return true;
        }
        let mut line = TextLine {
            number,
            text: String::new(),
            ending: span.ending,
            truncated: false,
        };
        match bytes {
            Some(bytes) => {
                let (text, truncated) = decode_capped(self.encoding, bytes, bytes.len());
                line.text = text;
                line.truncated = truncated;
            }
            None => self.unread.push((self.lines.len(), span)),
        }
        self.lines.push(line);
        self.line_no < self.last
    }
}
