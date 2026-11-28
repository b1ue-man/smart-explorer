//! `text.find` (api.md §4.15): the next or previous match in an open text
//! document, with the matcher of the content search (Notepad++ "Normal",
//! "Extended" and "Regular expression", "Match case", "Match whole word
//! only"), so a hit of "Suchen in Dateien" is found again in the view. The
//! document is searched in windows of lines; a match that runs across the
//! border of two windows is not found (a regular expression over line ends
//! inside a window is).
use super::error::ApiError;
use super::text_json::{byte_at_utf16, Utf16Cursor};
use crate::agent_proto::{search_reader, ContentEvent, ContentMatcher, ContentQuery, LineHit};
use crate::textview::LineProvider;
use serde_json::{json, Value};

/// Lines searched at once.
const WINDOW_LINES: usize = 2048;
/// Line text a hit may carry (the most the search allows).
const HIT_LINE_BYTES: u32 = 1 << 20;

/// One `text.find` request; `from_column` is a UTF-16 offset in `from_line`.
pub(crate) struct FindRequest {
    pub query: ContentQuery,
    /// Zero-based line the search starts in.
    pub from_line: u64,
    pub from_column: usize,
    pub backward: bool,
    /// Continue at the other end of the document when nothing is found.
    pub wrap: bool,
}

/// A match: zero-based line, UTF-16 range in that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub line: u64,
    pub start: usize,
    pub end: usize,
    /// Found only after continuing at the other end.
    pub wrapped: bool,
}

impl Found {
    pub(crate) fn json(found: Option<Found>) -> Value {
        match found {
            Some(found) => json!({
                "found": true,
                "line": found.line,
                "start": found.start,
                "end": found.end,
                "wrapped": found.wrapped,
            }),
            None => json!({ "found": false }),
        }
    }
}

/// A match in bytes of its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ByteMatch {
    line: u64,
    start: usize,
    end: usize,
}

pub(crate) fn find(
    lines: &(impl LineProvider + ?Sized),
    request: &FindRequest,
) -> Result<Option<Found>, ApiError> {
    let mut query = request.query.clone();
    query.context_lines = 0;
    query.max_hits = 0;
    query.files_only = false;
    query.max_line_bytes = HIT_LINE_BYTES;
    let matcher = ContentMatcher::new(&query).map_err(ApiError::invalid)?;
    let search = Search {
        lines,
        matcher: &matcher,
        query: &query,
    };
    let origin_text = lines
        .provide_lines(request.from_line, 1)?
        .into_iter()
        .next()
        .unwrap_or_default();
    let origin = (
        request.from_line,
        byte_at_utf16(&origin_text, request.from_column),
    );
    let (found, wrapped) = if request.backward {
        match search.last_before(origin)? {
            Some(found) => (Some(found), false),
            None if request.wrap => (search.last_from(origin)?, true),
            None => (None, false),
        }
    } else {
        match search.first_from(origin)? {
            Some(found) => (Some(found), false),
            None if request.wrap => (search.first_before(origin)?, true),
            None => (None, false),
        }
    };
    let Some(found) = found else {
        return Ok(None);
    };
    let text = lines
        .provide_lines(found.line, 1)?
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut cursor = Utf16Cursor::new(&text);
    Ok(Some(Found {
        line: found.line,
        start: cursor.at(found.start),
        end: cursor.at(found.end),
        wrapped,
    }))
}

struct Search<'a, L: LineProvider + ?Sized> {
    lines: &'a L,
    matcher: &'a ContentMatcher,
    query: &'a ContentQuery,
}

impl<L: LineProvider + ?Sized> Search<'_, L> {
    /// The matches of lines `first .. first + count` (fewer at the end) and
    /// the number of lines read.
    fn window(&self, first: u64, count: usize) -> Result<(Vec<ByteMatch>, usize), ApiError> {
        let texts = self.lines.provide_lines(first, count)?;
        if texts.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let joined = texts.join("\n");
        let mut hits: Vec<LineHit> = Vec::new();
        let mut emit = |event: ContentEvent| {
            if let ContentEvent::Hits { lines, .. } = event {
                hits.extend(lines.into_iter().filter(|line| !line.context));
            }
        };
        let mut reader = joined.as_bytes();
        search_reader(
            self.matcher,
            self.query,
            "",
            joined.len() as u64,
            0,
            &mut reader,
            &mut emit,
        )?;
        let mut out = Vec::new();
        for hit in hits {
            let Some(index) = hit.line.checked_sub(1) else {
                continue;
            };
            let Some(text) = texts.get(index as usize) else {
                continue;
            };
            // A cut line carries its text around the match: find the cut.
            let base = if hit.truncated {
                text.find(&hit.text).unwrap_or(0)
            } else {
                0
            };
            for (start, end) in &hit.ranges {
                out.push(ByteMatch {
                    line: first + index,
                    start: base + *start as usize,
                    end: base + *end as usize,
                });
            }
        }
        out.sort();
        Ok((out, texts.len()))
    }

    /// The first match at or after `origin`.
    fn first_from(&self, origin: (u64, usize)) -> Result<Option<ByteMatch>, ApiError> {
        let mut first = origin.0;
        loop {
            let (matches, read) = self.window(first, WINDOW_LINES)?;
            if let Some(found) = matches.into_iter().find(|m| (m.line, m.start) >= origin) {
                return Ok(Some(found));
            }
            if read < WINDOW_LINES {
                return Ok(None);
            }
            first += read as u64;
        }
    }

    /// The first match before `origin` (from the top of the document).
    fn first_before(&self, origin: (u64, usize)) -> Result<Option<ByteMatch>, ApiError> {
        let mut first = 0;
        while first <= origin.0 {
            let (matches, read) = self.window(first, WINDOW_LINES)?;
            if let Some(found) = matches.into_iter().find(|m| (m.line, m.start) < origin) {
                return Ok(Some(found));
            }
            if read < WINDOW_LINES {
                break;
            }
            first += read as u64;
        }
        Ok(None)
    }

    /// The last match that starts before `origin`.
    fn last_before(&self, origin: (u64, usize)) -> Result<Option<ByteMatch>, ApiError> {
        let mut end = origin.0 + 1;
        loop {
            let first = end.saturating_sub(WINDOW_LINES as u64);
            let (matches, _) = self.window(first, (end - first) as usize)?;
            if let Some(found) = matches
                .into_iter()
                .rev()
                .find(|m| (m.line, m.start) < origin)
            {
                return Ok(Some(found));
            }
            if first == 0 {
                return Ok(None);
            }
            end = first;
        }
    }

    /// The last match at or after `origin` (up to the end of the document).
    fn last_from(&self, origin: (u64, usize)) -> Result<Option<ByteMatch>, ApiError> {
        let mut first = origin.0;
        let mut last = None;
        loop {
            let (matches, read) = self.window(first, WINDOW_LINES)?;
            if let Some(found) = matches
                .into_iter()
                .rev()
                .find(|m| (m.line, m.start) >= origin)
            {
                last = Some(found);
            }
            if read < WINDOW_LINES {
                return Ok(last);
            }
            first += read as u64;
        }
    }
}
