//! Wire layout of the content search (`content_search_v1`, tags 93..=97;
//! from protocol 14 the flag `FLAG_IGNORE_FILES`, `content_search_ignore_v1`):
//! the request `ContentSearch`, the events (`Hits` 94 and `Skipped` 95 are
//! charged stream frames, `Progress` 96 is free) and the final `ContentDone`.
//! Little-endian, length-prefixed like every other frame.
use std::io;

use super::super::super::content_query::{
    ContentEvent, ContentQuery, ContentSummary, LineHit, SkipReason, TextMode,
};
use super::super::super::types::Frame;
use super::super::{bad, Reader};
use super::{
    add_len, optional_string_len, put_bool, put_i64, put_opt_str, put_str, put_u32, put_u64,
    string_len,
};

pub(super) const TAG_SEARCH: u8 = 93;
pub(super) const TAG_HITS: u8 = 94;
pub(super) const TAG_SKIPPED: u8 = 95;
pub(super) const TAG_PROGRESS: u8 = 96;
pub(super) const TAG_DONE: u8 = 97;

const FLAG_MATCH_CASE: u8 = 1;
const FLAG_WHOLE_WORD: u8 = 2;
const FLAG_DOT_NEWLINE: u8 = 4;
const FLAG_SUBFOLDERS: u8 = 8;
const FLAG_HIDDEN: u8 = 16;
const FLAG_FILES_ONLY: u8 = 32;
/// `ContentQuery::respect_ignore_files` (protocol 14, `content_search_ignore_v1`).
const FLAG_IGNORE_FILES: u8 = 64;
/// Every flag this build knows; another bit is refused.
const KNOWN_FLAGS: u8 = 0x7F;
const LINE_TRUNCATED: u8 = 1;
const LINE_CONTEXT: u8 = 2;
/// Smallest encoded line: number, offset, flags, text length, range count.
const MIN_LINE_BYTES: usize = 25;

/// Encoded body length after the request id and tag; `None` for other frames.
pub(in super::super) fn payload_len(frame: &Frame) -> Option<io::Result<usize>> {
    Some(match frame {
        Frame::ContentSearch { root, query } => search_len(root, query),
        Frame::ContentEvent(ContentEvent::Hits { rel, lines, .. }) => hits_len(rel, lines),
        Frame::ContentEvent(ContentEvent::Skipped { rel, reason }) => {
            let text = match reason {
                SkipReason::Binary => Ok(0),
                SkipReason::Unreadable(text) | SkipReason::UnlistableFolder(text) => {
                    string_len(text)
                }
            };
            text.and_then(|text| sum(&[string_len(rel)?, 1, text]))
        }
        Frame::ContentEvent(ContentEvent::Progress { .. }) => Ok(24),
        Frame::ContentDone(_) => Ok(41),
        _ => return None,
    })
}

fn sum(parts: &[usize]) -> io::Result<usize> {
    let mut length = 0;
    for part in parts {
        add_len(&mut length, *part)?;
    }
    Ok(length)
}

fn search_len(root: &str, query: &ContentQuery) -> io::Result<usize> {
    sum(&[
        string_len(root)?,
        string_len(&query.pattern)?,
        2,
        string_len(&query.filters)?,
        optional_string_len(&query.folder_include)?,
        optional_string_len(&query.folder_exclude)?,
        16,
    ])
}

fn hits_len(rel: &str, lines: &[LineHit]) -> io::Result<usize> {
    u32::try_from(lines.len()).map_err(|_| bad("frame too large"))?;
    let mut length = sum(&[string_len(rel)?, 21])?;
    for line in lines {
        u32::try_from(line.ranges.len()).map_err(|_| bad("frame too large"))?;
        add_len(&mut length, 17)?;
        add_len(&mut length, string_len(&line.text)?)?;
        add_len(&mut length, 4)?;
        let ranges = line
            .ranges
            .len()
            .checked_mul(8)
            .ok_or_else(|| bad("frame too large"))?;
        add_len(&mut length, ranges)?;
    }
    Ok(length)
}

/// Append the tag and fields; false for other frames.
pub(in super::super) fn encode(frame: &Frame, b: &mut Vec<u8>) -> bool {
    match frame {
        Frame::ContentSearch { root, query } => {
            b.push(TAG_SEARCH);
            put_str(b, root);
            put_str(b, &query.pattern);
            b.push(mode_byte(query.mode));
            b.push(query_flags(query));
            put_str(b, &query.filters);
            put_opt_str(b, &query.folder_include);
            put_opt_str(b, &query.folder_exclude);
            put_u32(b, query.context_lines);
            put_u64(b, query.max_hits);
            put_u32(b, query.max_line_bytes);
        }
        Frame::ContentEvent(ContentEvent::Hits {
            rel,
            size,
            mtime_ms,
            lines,
            last,
        }) => {
            b.push(TAG_HITS);
            put_str(b, rel);
            put_u64(b, *size);
            put_i64(b, *mtime_ms);
            put_bool(b, *last);
            put_u32(b, lines.len() as u32);
            for line in lines {
                put_u64(b, line.line);
                put_u64(b, line.offset);
                let mut flags = 0;
                if line.truncated {
                    flags |= LINE_TRUNCATED;
                }
                if line.context {
                    flags |= LINE_CONTEXT;
                }
                b.push(flags);
                put_str(b, &line.text);
                put_u32(b, line.ranges.len() as u32);
                for (start, end) in &line.ranges {
                    put_u32(b, *start);
                    put_u32(b, *end);
                }
            }
        }
        Frame::ContentEvent(ContentEvent::Skipped { rel, reason }) => {
            b.push(TAG_SKIPPED);
            put_str(b, rel);
            match reason {
                SkipReason::Binary => b.push(0),
                SkipReason::Unreadable(text) => {
                    b.push(1);
                    put_str(b, text);
                }
                SkipReason::UnlistableFolder(text) => {
                    b.push(2);
                    put_str(b, text);
                }
            }
        }
        Frame::ContentEvent(ContentEvent::Progress { files, bytes, hits }) => {
            b.push(TAG_PROGRESS);
            put_u64(b, *files);
            put_u64(b, *bytes);
            put_u64(b, *hits);
        }
        Frame::ContentDone(summary) => {
            b.push(TAG_DONE);
            for value in [
                summary.files_searched,
                summary.files_matched,
                summary.hits,
                summary.bytes,
                summary.skipped,
            ] {
                put_u64(b, value);
            }
            put_bool(b, summary.limited);
        }
        _ => return false,
    }
    true
}

fn mode_byte(mode: TextMode) -> u8 {
    match mode {
        TextMode::Normal => 0,
        TextMode::Extended => 1,
        TextMode::Regex => 2,
    }
}

fn query_flags(query: &ContentQuery) -> u8 {
    [
        (query.match_case, FLAG_MATCH_CASE),
        (query.whole_word, FLAG_WHOLE_WORD),
        (query.dot_matches_newline, FLAG_DOT_NEWLINE),
        (query.subfolders, FLAG_SUBFOLDERS),
        (query.hidden, FLAG_HIDDEN),
        (query.files_only, FLAG_FILES_ONLY),
        (query.respect_ignore_files, FLAG_IGNORE_FILES),
    ]
    .into_iter()
    .filter(|(set, _)| *set)
    .fold(0, |flags, (_, flag)| flags | flag)
}

/// Decode a frame of this set; `None` for an unknown tag.
pub(in super::super) fn decode(tag: u8, r: &mut Reader) -> io::Result<Option<Frame>> {
    Ok(Some(match tag {
        TAG_SEARCH => {
            let root = r.string()?;
            let pattern = r.string()?;
            let mode = match r.u8()? {
                0 => TextMode::Normal,
                1 => TextMode::Extended,
                2 => TextMode::Regex,
                _ => return Err(bad("unknown content search mode")),
            };
            let flags = r.u8()?;
            if flags & !KNOWN_FLAGS != 0 {
                return Err(bad("unknown content search flags"));
            }
            Frame::ContentSearch {
                root,
                query: ContentQuery {
                    pattern,
                    mode,
                    match_case: flags & FLAG_MATCH_CASE != 0,
                    whole_word: flags & FLAG_WHOLE_WORD != 0,
                    dot_matches_newline: flags & FLAG_DOT_NEWLINE != 0,
                    subfolders: flags & FLAG_SUBFOLDERS != 0,
                    hidden: flags & FLAG_HIDDEN != 0,
                    files_only: flags & FLAG_FILES_ONLY != 0,
                    filters: r.string()?,
                    folder_include: r.opt_str()?,
                    folder_exclude: r.opt_str()?,
                    context_lines: r.u32()?,
                    max_hits: r.u64()?,
                    max_line_bytes: r.u32()?,
                    respect_ignore_files: flags & FLAG_IGNORE_FILES != 0,
                },
            }
        }
        TAG_HITS => {
            let rel = r.string()?;
            let size = r.u64()?;
            let mtime_ms = r.i64()?;
            let last = r.bool()?;
            let count = r.u32()? as usize;
            if count > r.remaining() / MIN_LINE_BYTES {
                return Err(bad("content hit count exceeds the remaining frame bytes"));
            }
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                lines.push(decode_line(r)?);
            }
            Frame::ContentEvent(ContentEvent::Hits {
                rel,
                size,
                mtime_ms,
                lines,
                last,
            })
        }
        TAG_SKIPPED => {
            let rel = r.string()?;
            let reason = match r.u8()? {
                0 => SkipReason::Binary,
                1 => SkipReason::Unreadable(r.string()?),
                2 => SkipReason::UnlistableFolder(r.string()?),
                _ => return Err(bad("unknown content skip reason")),
            };
            Frame::ContentEvent(ContentEvent::Skipped { rel, reason })
        }
        TAG_PROGRESS => Frame::ContentEvent(ContentEvent::Progress {
            files: r.u64()?,
            bytes: r.u64()?,
            hits: r.u64()?,
        }),
        TAG_DONE => Frame::ContentDone(ContentSummary {
            files_searched: r.u64()?,
            files_matched: r.u64()?,
            hits: r.u64()?,
            bytes: r.u64()?,
            skipped: r.u64()?,
            limited: r.bool()?,
        }),
        _ => return Ok(None),
    }))
}

fn decode_line(r: &mut Reader) -> io::Result<LineHit> {
    let line = r.u64()?;
    let offset = r.u64()?;
    let flags = r.u8()?;
    if flags & !(LINE_TRUNCATED | LINE_CONTEXT) != 0 {
        return Err(bad("unknown content line flags"));
    }
    let text = r.string()?;
    let count = r.u32()? as usize;
    if count > r.remaining() / 8 {
        return Err(bad("content range count exceeds the remaining frame bytes"));
    }
    let mut ranges = Vec::with_capacity(count);
    for _ in 0..count {
        ranges.push((r.u32()?, r.u32()?));
    }
    Ok(LineHit {
        line,
        offset,
        text,
        ranges,
        truncated: flags & LINE_TRUNCATED != 0,
        context: flags & LINE_CONTEXT != 0,
    })
}

#[cfg(test)]
#[path = "content_frames_tests.rs"]
mod tests;
