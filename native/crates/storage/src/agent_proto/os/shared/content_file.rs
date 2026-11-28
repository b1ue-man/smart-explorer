//! Searching the content of one file (`ContentRun::search_reader`) with
//! ripgrep's `grep-searcher`: line numbers, context lines, binary detection
//! (a NUL byte: `SkipReason::Binary`), BOM sniffing (UTF-16 files are searched
//! decoded), the cut of long lines, `files_only` and the hit budget of
//! `max_hits`. A file without BOM that is no valid UTF-8 and has no hit is
//! searched a second time transcoded from Windows-1252 when the pattern has
//! characters beyond ASCII, as Notepad++ finds text in ANSI files.
use std::cell::{Cell, RefCell};
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use grep_matcher::LineTerminator;
use grep_searcher::{
    BinaryDetection, Encoding, Searcher, SearcherBuilder, Sink, SinkContext, SinkFinish, SinkMatch,
};

use super::content_lines::{block_lines, clip_ranges, line_hit, without_terminator};
use super::content_match::{ContentMatcher, Engine};
use super::content_query::{ContentEvent, ContentQuery, LineHit, SkipReason};
use super::content_run::{ContentRun, FileOutcome};
use super::content_source::{CountingReader, Tee, Utf8Probe, STOPPED};

/// Lines or text bytes after which a file's hits go out in one event.
const FLUSH_LINES: usize = 128;
const FLUSH_BYTES: usize = 256 * 1024;

/// What the searcher of a thread is configured with (one is kept per thread
/// and kind, so a file does not allocate a fresh 64 KiB buffer).
#[derive(Clone, Copy, PartialEq, Eq)]
struct SearcherKey {
    multi_line: bool,
    context: u32,
    ansi: bool,
    terminator: LineTerminator,
}

thread_local! {
    static SEARCHERS: RefCell<Vec<(SearcherKey, Searcher)>> = const { RefCell::new(Vec::new()) };
}

fn with_searcher<T>(key: SearcherKey, run: impl FnOnce(&mut Searcher) -> T) -> T {
    SEARCHERS.with(|cell| {
        let mut searchers = cell.borrow_mut();
        let index = match searchers.iter().position(|(known, _)| *known == key) {
            Some(index) => index,
            None => {
                if searchers.len() >= 4 {
                    searchers.remove(0);
                }
                searchers.push((key, build_searcher(key)));
                searchers.len() - 1
            }
        };
        run(&mut searchers[index].1)
    })
}

fn build_searcher(key: SearcherKey) -> Searcher {
    let mut builder = SearcherBuilder::new();
    builder
        .line_terminator(key.terminator)
        .line_number(true)
        .multi_line(key.multi_line)
        .before_context(key.context as usize)
        .after_context(key.context as usize)
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .bom_sniffing(true);
    if key.ansi {
        builder.encoding(Encoding::new("windows-1252").ok());
    }
    builder.build()
}

impl ContentRun<'_> {
    /// Searches the file `rel` (`size`, `mtime_ms` as listed) whose content
    /// `open` reads; `open` is called again for the Windows-1252 pass. Hits,
    /// a skip (unreadable, binary) and progress go to the run's receiver.
    /// `Err` only when the whole search ends here (canceled, receiver gone).
    pub fn search_reader<'o>(
        &self,
        rel: &str,
        size: u64,
        mtime_ms: i64,
        open: &mut dyn FnMut() -> io::Result<Box<dyn Read + 'o>>,
    ) -> io::Result<FileOutcome> {
        self.check()?;
        let first = open();
        let mut again = || Some(open());
        let mut out = |event| self.send(event);
        let searched = search_file(self, (rel, size, mtime_ms), first, &mut again, &mut out);
        if searched.binary {
            self.skip(rel, SkipReason::Binary);
        } else if let Some(error) = &searched.error {
            self.skip(rel, SkipReason::Unreadable(error.to_string()));
        } else if !searched.stopped || searched.hits > 0 {
            self.count_file(searched.hits);
        }
        self.check()?;
        Ok(FileOutcome {
            hits: searched.hits,
            bytes: searched.bytes,
            skipped: searched.binary || searched.error.is_some(),
        })
    }
}

/// Searches one file read through `reader` for a host that walks, counts and
/// limits on its own (the Share host): the hits of this file go to `emit`
/// (`query.max_hits` and `files_only` hold within the file), progress does
/// not. A binary file is `skipped` in the outcome, not an event; a read
/// error is the `Err`. The bytes are kept (up to 16 MiB) for the
/// Windows-1252 pass of an ANSI file.
pub fn search_reader(
    matcher: &ContentMatcher,
    query: &ContentQuery,
    rel: &str,
    size: u64,
    mtime_ms: i64,
    reader: &mut dyn Read,
    emit: &mut dyn FnMut(ContentEvent),
) -> io::Result<FileOutcome> {
    let idle = AtomicBool::new(false);
    // Progress of the file stays here: the caller counts on its own.
    let progress = |_: ContentEvent| true;
    let run = ContentRun::new(matcher, query, &progress, &idle);
    let kept = RefCell::new(matcher.non_ascii().then(Vec::new));
    let read = Cell::new(0u64);
    let first: Box<dyn Read + '_> = Box::new(Tee {
        inner: reader,
        kept: &kept,
        read: &read,
    });
    let mut again = || {
        let bytes = kept.borrow_mut().take()?;
        let reader: Box<dyn Read + '_> = Box::new(io::Cursor::new(bytes));
        Some(Ok(reader))
    };
    let mut out = |event| {
        emit(event);
        true
    };
    let searched = search_file(&run, (rel, size, mtime_ms), Ok(first), &mut again, &mut out);
    if let Some(error) = searched.error {
        return Err(error);
    }
    Ok(FileOutcome {
        hits: searched.hits,
        bytes: read.get(),
        skipped: searched.binary,
    })
}

/// What searching one file produced.
struct Searched {
    hits: u64,
    bytes: u64,
    binary: bool,
    /// The search stopped inside the file (cancel, `max_hits`, receiver gone).
    stopped: bool,
    error: Option<io::Error>,
}

/// Searches one file: `first` is its content, `again` gives it once more for
/// the Windows-1252 pass (`None`: not available). The file's hits go to
/// `out` (false = the receiver is gone), its last event marked.
fn search_file<'o>(
    run: &ContentRun<'_>,
    (rel, size, mtime_ms): (&str, u64, i64),
    first: io::Result<Box<dyn Read + 'o>>,
    again: &mut dyn FnMut() -> Option<io::Result<Box<dyn Read + 'o>>>,
    out: &mut dyn FnMut(ContentEvent) -> bool,
) -> Searched {
    let bytes = Cell::new(0u64);
    let mut file = FileSink::new(run, (rel, size, mtime_ms), out);
    let probe = run.matcher.non_ascii();
    let mut candidate = false;
    let mut result = first.and_then(|reader| {
        let mut reader = Utf8Probe::new(CountingReader::new(reader, run, &bytes), probe);
        let result = file.search(&mut reader, false);
        candidate = probe && reader.ansi_candidate();
        result
    });
    if result.is_ok() && candidate && file.hits == 0 && !file.binary && !run.halted() {
        if let Some(reader) = again() {
            result = reader.and_then(|reader| {
                file.search(&mut CountingReader::new(reader, run, &bytes), true)
            });
        }
    }
    let stopped = result
        .as_ref()
        .is_err_and(|error| error.to_string() == STOPPED);
    if file.binary && !file.flushed {
        run.refund(file.hits);
        file.hits = 0;
        file.lines.clear();
    }
    if file.hits > 0 {
        let lines = std::mem::take(&mut file.lines);
        file.emit(lines, true);
    }
    Searched {
        hits: file.hits,
        bytes: bytes.get(),
        binary: file.binary,
        stopped,
        error: result.err().filter(|_| !stopped),
    }
}

/// The hits of one file while it is searched.
struct FileSink<'r, 'q> {
    run: &'r ContentRun<'q>,
    rel: &'r str,
    size: u64,
    mtime_ms: i64,
    out: &'r mut dyn FnMut(ContentEvent) -> bool,
    lines: Vec<LineHit>,
    pending: usize,
    hits: u64,
    flushed: bool,
    binary: bool,
}

impl<'r, 'q> FileSink<'r, 'q> {
    fn new(
        run: &'r ContentRun<'q>,
        (rel, size, mtime_ms): (&'r str, u64, i64),
        out: &'r mut dyn FnMut(ContentEvent) -> bool,
    ) -> Self {
        Self {
            run,
            rel,
            size,
            mtime_ms,
            out,
            lines: Vec::new(),
            pending: 0,
            hits: 0,
            flushed: false,
            binary: false,
        }
    }

    fn search(&mut self, reader: &mut dyn Read, ansi: bool) -> io::Result<()> {
        let matcher = self.run.matcher;
        let key = SearcherKey {
            multi_line: matcher.multi_line(),
            context: if self.run.query.files_only {
                0
            } else {
                self.run.query.context_lines
            },
            ansi,
            terminator: matcher.line_terminator(),
        };
        with_searcher(key, |searcher| match matcher.engine() {
            Engine::Regex(regex) => searcher.search_reader(regex, reader, &mut *self),
            Engine::Fancy(fancy) => searcher.search_reader(fancy, reader, &mut *self),
        })
    }

    fn push(&mut self, line: LineHit) {
        self.pending += line.text.len() + line.ranges.len() * 8 + 32;
        self.lines.push(line);
        if self.lines.len() >= FLUSH_LINES || self.pending >= FLUSH_BYTES {
            let lines = std::mem::take(&mut self.lines);
            self.emit(lines, false);
        }
    }

    fn emit(&mut self, lines: Vec<LineHit>, last: bool) {
        self.pending = 0;
        self.flushed = true;
        (self.out)(ContentEvent::Hits {
            rel: self.rel.to_string(),
            size: self.size,
            mtime_ms: self.mtime_ms,
            lines,
            last,
        });
    }
}

impl Sink for FileSink<'_, '_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, found: &SinkMatch<'_>) -> io::Result<bool> {
        if self.binary || self.run.halted() {
            return Ok(false);
        }
        let block = found.bytes();
        let ranges = self.run.matcher.ranges(block)?;
        let first = found.line_number().unwrap_or(0);
        let base = found.absolute_byte_offset();
        let limit = self.run.query.line_bytes();
        for (index, (start, end)) in block_lines(block).into_iter().enumerate() {
            if !self.run.take_hit() {
                return Ok(false);
            }
            self.hits += 1;
            let content = without_terminator(&block[start..end]);
            let line_ranges = clip_ranges(&ranges, (start, start + content.len()));
            let position = (first + index as u64, base + start as u64);
            self.push(line_hit(position, content, &line_ranges, limit, false));
            if self.run.query.files_only {
                return Ok(false);
            }
        }
        Ok(!self.run.halted())
    }

    fn context(&mut self, _searcher: &Searcher, context: &SinkContext<'_>) -> io::Result<bool> {
        let block = context.bytes();
        let first = context.line_number().unwrap_or(0);
        let base = context.absolute_byte_offset();
        let limit = self.run.query.line_bytes();
        for (index, (start, end)) in block_lines(block).into_iter().enumerate() {
            let content = without_terminator(&block[start..end]);
            let position = (first + index as u64, base + start as u64);
            self.push(line_hit(position, content, &[], limit, true));
        }
        Ok(!self.run.halted())
    }

    /// A NUL byte: the file is not text; nothing more of it is searched.
    fn binary_data(&mut self, _searcher: &Searcher, _offset: u64) -> io::Result<bool> {
        self.binary = true;
        Ok(false)
    }

    fn finish(&mut self, _searcher: &Searcher, finish: &SinkFinish) -> io::Result<()> {
        if finish.binary_byte_offset().is_some() {
            self.binary = true;
        }
        Ok(())
    }
}
