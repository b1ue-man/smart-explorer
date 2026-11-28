//! `read` by lines (`line`, `lines`): numbered text as `cat -n` prints it
//! (the line number right-aligned in six columns, a tab, the line without its
//! line end), one page within the answer budget, continued with
//! `line` = `next_line`. The file is streamed from its start; a line longer
//! than the page keeps only its start (`cut_lines`), so no line is held in
//! memory beyond the page budget.
use super::args::Args;
use super::file_tools::etag;
use super::response;
use super::session::Session;
use super::targets::Resolved;
use crate::vfs::VfsMeta;
use rmcp::model::CallToolResult;
use serde_json::json;
use std::io::{self, BufRead, BufReader};

pub(super) fn read(
    session: &Session,
    target: &Resolved,
    meta: &VfsMeta,
    through_link: bool,
    args: &Args<'_>,
) -> Result<CallToolResult, String> {
    for key in ["offset", "length"] {
        if args.value(key).is_some() {
            return Err(format!(
                "`{key}` counts bytes; with `line`/`lines` the page is counted in lines"
            ));
        }
    }
    match args.opt_string("encoding")?.as_deref() {
        None | Some("text") => {}
        Some(_) => {
            return Err(
                "`line`/`lines` read text; read bytes as base64 with `offset` instead".into(),
            )
        }
    }
    let first = args.opt_u64("line")?.unwrap_or(1);
    if first == 0 {
        return Err("`line` counts from 1".into());
    }
    let wanted = args.opt_u64("lines")?;
    if wanted == Some(0) {
        return Err("`lines` must be at least 1".into());
    }
    let budget = response::page_budget(args.opt_u64("max_bytes")?);
    let known_size = (!through_link).then_some(meta.size);
    let tag = (!through_link).then(|| etag(meta));
    let reader = target
        .backend
        .open_read_id(&target.path, meta.id.as_deref())
        .map_err(|error| {
            let text = session.targets.io_error(target, error);
            if through_link {
                format!(
                    "{} is a link whose target does not exist, is a directory, or cannot be read: {text}",
                    target.path
                )
            } else {
                text
            }
        })?;
    let page = collect(
        BufReader::with_capacity(64 * 1024, reader),
        first,
        wanted,
        budget,
    )
    .map_err(|error| session.targets.io_error(target, error))?;
    if page.binary {
        return Ok(response::error_with(
            format!(
                "{} looks binary; read it with encoding \"base64\" or copy it to a local path",
                target.path
            ),
            json!({ "size": known_size, "etag": tag }),
        ));
    }
    Ok(response::ok(json!({
        "path": target.path,
        "size": known_size,
        "etag": tag,
        "through_link": through_link,
        "line": first,
        "lines": page.count,
        "encoding": "text",
        "content": page.content,
        "next_line": page.next_line,
        "total_lines": page.total_lines,
        "crlf": page.crlf,
        "lossy": page.lossy,
        "cut_lines": page.cut,
    })))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct LinePage {
    /// `{number:>6}\t{text}\n` per line.
    pub(super) content: String,
    pub(super) count: u64,
    /// The first line after the page; `None` at the end of the file.
    pub(super) next_line: Option<u64>,
    /// The number of lines, once the end of the file was read.
    pub(super) total_lines: Option<u64>,
    /// A line of the page ended with CR LF (shown without the CR).
    pub(super) crlf: bool,
    /// Invalid UTF-8 was replaced with U+FFFD.
    pub(super) lossy: bool,
    /// Lines longer than the page; only their start is shown.
    pub(super) cut: Vec<u64>,
    /// A NUL byte was read: not a text file.
    pub(super) binary: bool,
}

/// The page of up to `wanted` lines from line `first` within `budget` bytes.
pub(super) fn collect(
    mut reader: impl BufRead,
    first: u64,
    wanted: Option<u64>,
    budget: usize,
) -> io::Result<LinePage> {
    let mut page = LinePage::default();
    let mut number = 0u64;
    let mut line = Vec::new();
    let mut full = false;
    loop {
        let (consumed, dropped) = next_line(&mut reader, &mut line, budget, &mut page.binary)?;
        if page.binary {
            return Ok(page);
        }
        if consumed == 0 {
            page.total_lines = Some(number);
            return Ok(page);
        }
        number += 1;
        if number < first {
            continue;
        }
        if full || wanted.is_some_and(|wanted| page.count >= wanted) {
            page.next_line = Some(number);
            return Ok(page);
        }
        let mut cut = dropped;
        if !dropped && line.ends_with(b"\n") {
            line.pop();
            if line.ends_with(b"\r") {
                line.pop();
                page.crlf = true;
            }
        }
        let prefix = format!("{number:>6}\t");
        let room = budget.saturating_sub(page.content.len() + prefix.len() + 1);
        if line.len() > room {
            if page.count > 0 {
                page.next_line = Some(number);
                return Ok(page);
            }
            // A single line larger than the page: its start.
            line.truncate(room);
            cut = true;
            full = true;
        }
        if cut {
            // Leave out a character the cut split.
            if let Err(error) = std::str::from_utf8(&line) {
                if error.error_len().is_none() {
                    line.truncate(error.valid_up_to());
                }
            }
            page.cut.push(number);
        }
        let text = String::from_utf8_lossy(&line);
        page.lossy |= matches!(text, std::borrow::Cow::Owned(_));
        page.content.push_str(&prefix);
        page.content.push_str(&text);
        page.content.push('\n');
        page.count += 1;
    }
}

/// Reads one line into `line`, keeping at most `keep` bytes of it; returns
/// the bytes consumed (0 at the end) and whether bytes were dropped. Sets
/// `nul` when the line holds a NUL byte.
fn next_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    keep: usize,
    nul: &mut bool,
) -> io::Result<(usize, bool)> {
    line.clear();
    let mut consumed = 0;
    let mut dropped = false;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if chunk.is_empty() {
            return Ok((consumed, dropped));
        }
        let (part, done) = match chunk.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&chunk[..=end], true),
            None => (chunk, false),
        };
        *nul |= part.contains(&0);
        let room = keep.saturating_sub(line.len());
        if part.len() > room {
            line.extend_from_slice(&part[..room]);
            dropped = true;
        } else {
            line.extend_from_slice(part);
        }
        let length = part.len();
        reader.consume(length);
        consumed += length;
        if done || *nul {
            return Ok((consumed, dropped));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect;
    use std::io::Cursor;

    fn page(text: &[u8], first: u64, wanted: Option<u64>, budget: usize) -> super::LinePage {
        collect(Cursor::new(text.to_vec()), first, wanted, budget).unwrap()
    }

    #[test]
    fn mcp_root_task_read_by_lines_numbers_lines_like_cat_n() {
        let text = b"alpha\r\nbeta\n\ngamma";
        let all = page(text, 1, None, 1024);
        assert_eq!(
            all.content,
            "     1\talpha\n     2\tbeta\n     3\t\n     4\tgamma\n"
        );
        assert_eq!(all.count, 4);
        assert_eq!(all.next_line, None);
        assert_eq!(all.total_lines, Some(4));
        assert!(all.crlf);
        // A window of lines continues with `next_line`.
        let window = page(text, 2, Some(2), 1024);
        assert_eq!(window.content, "     2\tbeta\n     3\t\n");
        assert_eq!(window.next_line, Some(4));
        assert_eq!(window.total_lines, None);
        let rest = page(text, 4, Some(10), 1024);
        assert_eq!(rest.content, "     4\tgamma\n");
        assert_eq!(rest.next_line, None);
        // Beyond the end: nothing, and the number of lines.
        let beyond = page(text, 9, None, 1024);
        assert_eq!((beyond.count, beyond.total_lines), (0, Some(4)));
        // A trailing line end does not start another line.
        assert_eq!(page(b"a\nb\n", 1, None, 1024).total_lines, Some(2));
        assert_eq!(page(b"", 1, None, 1024).total_lines, Some(0));
    }

    #[test]
    fn mcp_root_task_read_by_lines_stays_within_the_budget() {
        let text = "0123456789\n".repeat(100);
        // Each entry is 7 + 10 + 1 = 18 bytes: 5 fit into 90.
        let first = page(text.as_bytes(), 1, None, 90);
        assert_eq!(first.count, 5);
        assert_eq!(first.next_line, Some(6));
        assert!(first.content.len() <= 90);
        // A line longer than the page shows its start and is marked.
        let long = format!("{}\nnext\n", "ä".repeat(100));
        let cut = page(long.as_bytes(), 1, None, 64);
        assert_eq!(cut.count, 1);
        assert_eq!(cut.cut, [1]);
        assert_eq!(cut.next_line, Some(2));
        assert!(cut.content.len() <= 64, "{}", cut.content.len());
        assert!(!cut.lossy, "a split character is left out, not replaced");
        // Invalid UTF-8 is replaced and reported; a NUL byte is binary.
        assert!(page(b"a\xff\n", 1, None, 1024).lossy);
        assert!(page(b"text\nbin\0ary\n", 1, None, 1024).binary);
    }
}
