//! Output records of a content search, shared by `se grep` and the MCP
//! `grep` tool: `rel:line:text` per hit (context lines `rel-line-text`,
//! groups split by `--`), the names of files with a hit, or `rel:count`;
//! with `json` one JSON object per line (`rel`, `line`, `text`, `ranges`).
//! Also the search thread and the Notepad++ totals line.
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary, LineHit};
use crate::vfs::BackendHandle;
use serde_json::json;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// What a content search prints per file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::cli) enum GrepOutput {
    /// Every hit line with its number (and context lines).
    Content,
    /// The names of files with a hit.
    Files,
    /// The number of hit lines per file.
    Count,
}

impl GrepOutput {
    pub(in crate::cli) fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "content" => Some(Self::Content),
            "files" => Some(Self::Files),
            "count" => Some(Self::Count),
            _ => None,
        }
    }

    pub(in crate::cli) fn key(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Files => "files",
            Self::Count => "count",
        }
    }
}

/// Runs the search on its own thread and hands every event to `on_event` on
/// the calling one; a cancelled search ends at its next check.
pub(in crate::cli) fn run_search(
    backend: BackendHandle,
    root: String,
    query: ContentQuery,
    cancel: Arc<AtomicBool>,
    on_event: &mut dyn FnMut(ContentEvent),
) -> io::Result<ContentSummary> {
    let (sender, events) = crossbeam_channel::bounded::<ContentEvent>(1_024);
    let worker = std::thread::Builder::new()
        .name("se-grep".into())
        .spawn(move || crate::vfs::content_search(&*backend, &root, &query, sender, &cancel))?;
    // The channel closes when the search returns (it owns the sender).
    for event in events.iter() {
        on_event(event);
    }
    worker
        .join()
        .map_err(|_| io::Error::other("the content search panicked"))?
}

/// `Suche „x“ (N Treffer in M Dateien von K durchsuchten)`, Notepad++'s totals
/// ("1 hit in 1 file of 1 searched"), worded as the desktop and the app word it.
pub(in crate::cli) fn totals_line(pattern: &str, summary: &ContentSummary) -> String {
    let files = if summary.files_matched == 1 {
        "Datei"
    } else {
        "Dateien"
    };
    let mut line = format!(
        "Suche „{pattern}“ ({} Treffer in {} {files} von {} durchsuchten",
        summary.hits, summary.files_matched, summary.files_searched
    );
    if summary.skipped > 0 {
        line.push_str(&format!("; {} übersprungen", summary.skipped));
    }
    if summary.limited {
        line.push_str("; bei der Höchstzahl der Treffer beendet");
    }
    line.push(')');
    line
}

/// Turns the hits of a running search into output records.
pub(in crate::cli) struct Records {
    output: GrepOutput,
    json: bool,
    color: bool,
    context: bool,
    /// The last printed line (file, number), for the `--` between groups.
    last: Option<(String, u64)>,
    /// The file whose name was printed last (`Files`).
    named: Option<String>,
    /// Hit lines of the current file so far (`Count`).
    count: u64,
}

impl Records {
    pub(in crate::cli) fn new(output: GrepOutput, json: bool, color: bool, context: bool) -> Self {
        Self {
            output,
            json,
            color,
            context,
            last: None,
            named: None,
            count: 0,
        }
    }

    /// Appends the records of one `Hits` event of file `rel` to `out`.
    pub(in crate::cli) fn hits(
        &mut self,
        rel: &str,
        lines: &[LineHit],
        last: bool,
        out: &mut Vec<u8>,
    ) {
        match self.output {
            GrepOutput::Content => {
                for hit in lines {
                    self.line(rel, hit, out);
                }
            }
            GrepOutput::Files => {
                if self.named.as_deref() != Some(rel) && lines.iter().any(|hit| !hit.context) {
                    self.named = Some(rel.to_string());
                    let record = if self.json {
                        json!({ "rel": rel }).to_string()
                    } else if self.color {
                        format!("\x1b[35m{}\x1b[0m", clean(rel, true))
                    } else {
                        clean(rel, false)
                    };
                    out.extend_from_slice(record.as_bytes());
                    out.push(b'\n');
                }
            }
            GrepOutput::Count => {
                self.count += lines.iter().filter(|hit| !hit.context).count() as u64;
                if last {
                    let record = if self.json {
                        json!({ "rel": rel, "count": self.count }).to_string()
                    } else if self.color {
                        format!(
                            "\x1b[35m{}\x1b[0m\x1b[36m:\x1b[0m{}",
                            clean(rel, true),
                            self.count
                        )
                    } else {
                        format!("{}:{}", clean(rel, false), self.count)
                    };
                    out.extend_from_slice(record.as_bytes());
                    out.push(b'\n');
                    self.count = 0;
                }
            }
        }
    }

    fn line(&mut self, rel: &str, hit: &LineHit, out: &mut Vec<u8>) {
        if self.context {
            let gap = match &self.last {
                Some((file, number)) => file != rel || hit.line > number + 1,
                None => false,
            };
            if gap && !self.json {
                out.extend_from_slice(b"--\n");
            }
            self.last = Some((rel.to_string(), hit.line));
        }
        let record = if self.json {
            let mut value = json!({
                "rel": rel,
                "line": hit.line,
                "text": hit.text,
                "ranges": hit.ranges.iter().map(|(start, end)| [start, end]).collect::<Vec<_>>(),
            });
            if let serde_json::Value::Object(fields) = &mut value {
                if hit.context {
                    fields.insert("context".into(), json!(true));
                }
                if hit.truncated {
                    fields.insert("truncated".into(), json!(true));
                }
            }
            value.to_string()
        } else {
            text_line(rel, hit, self.color)
        };
        out.extend_from_slice(record.as_bytes());
        out.push(b'\n');
    }
}

/// `rel:line:text` (context lines `rel-line-text`); with `color` the hits are
/// highlighted as grep does (file magenta, number green, match bold red).
fn text_line(rel: &str, hit: &LineHit, color: bool) -> String {
    let separator = if hit.context { '-' } else { ':' };
    if !color {
        return format!(
            "{}{separator}{}{separator}{}",
            clean(rel, false),
            hit.line,
            clean(&hit.text, false)
        );
    }
    let mut text = String::new();
    let mut cursor = 0usize;
    let mut ranges = hit.ranges.clone();
    ranges.sort_unstable();
    for (start, end) in ranges {
        let (start, end) = (start as usize, (end as usize).min(hit.text.len()));
        if start < cursor
            || start >= end
            || !hit.text.is_char_boundary(start)
            || !hit.text.is_char_boundary(end)
        {
            continue;
        }
        text.push_str(&clean(&hit.text[cursor..start], true));
        text.push_str("\x1b[1;31m");
        text.push_str(&clean(&hit.text[start..end], true));
        text.push_str("\x1b[0m");
        cursor = end;
    }
    text.push_str(&clean(&hit.text[cursor..], true));
    format!(
        "\x1b[35m{}\x1b[0m\x1b[36m{separator}\x1b[0m\x1b[32m{}\x1b[0m\x1b[36m{separator}\x1b[0m{text}",
        clean(rel, true),
        hit.line
    )
}

/// One record per line: line ends inside a text become `\n`/`\r`; for a
/// terminal also every other control character (no escape sequence of a
/// file reaches the terminal), as U+FFFD.
fn clean(text: &str, terminal: bool) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' => cleaned.push_str("\\n"),
            '\r' => cleaned.push_str("\\r"),
            '\t' => cleaned.push('\t'),
            control if terminal && control.is_control() => {
                cleaned.push(char::REPLACEMENT_CHARACTER)
            }
            other => cleaned.push(other),
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::{totals_line, GrepOutput, Records};
    use crate::agent_proto::{ContentSummary, LineHit};

    fn hit(line: u64, text: &str, ranges: &[(u32, u32)], context: bool) -> LineHit {
        LineHit {
            line,
            offset: 0,
            text: text.into(),
            ranges: ranges.to_vec(),
            truncated: false,
            context,
        }
    }

    #[test]
    fn grep_task_cli_records_and_totals() {
        let mut out = Vec::new();
        let mut records = Records::new(GrepOutput::Content, false, false, true);
        records.hits(
            "a.rs",
            &[hit(1, "x", &[], true), hit(2, "foo bar", &[(0, 3)], false)],
            false,
            &mut out,
        );
        records.hits(
            "a.rs",
            &[hit(9, "foo\nnext", &[(0, 3)], false)],
            true,
            &mut out,
        );
        records.hits("b.rs", &[hit(4, "foo", &[(0, 3)], false)], true, &mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "a.rs-1-x\na.rs:2:foo bar\n--\na.rs:9:foo\\nnext\n--\nb.rs:4:foo\n"
        );
        let mut colored = Vec::new();
        Records::new(GrepOutput::Content, false, true, false).hits(
            "a.rs",
            &[hit(2, "a foo\u{1b}[2J", &[(2, 5)], false)],
            true,
            &mut colored,
        );
        let colored = String::from_utf8(colored).unwrap();
        assert!(colored.contains("\x1b[1;31mfoo\x1b[0m"), "{colored:?}");
        assert!(
            !colored.contains("\u{1b}[2J"),
            "a file's escape sequence never reaches the terminal"
        );
        let mut json = Vec::new();
        Records::new(GrepOutput::Content, true, false, false).hits(
            "a.rs",
            &[hit(2, "foo", &[(0, 3)], false)],
            true,
            &mut json,
        );
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"rel": "a.rs", "line": 2, "text": "foo", "ranges": [[0, 3]]})
        );
        let mut counts = Vec::new();
        let mut count = Records::new(GrepOutput::Count, false, false, false);
        count.hits(
            "a.rs",
            &[hit(1, "a", &[], false), hit(2, "b", &[], false)],
            false,
            &mut counts,
        );
        count.hits("a.rs", &[hit(5, "c", &[], false)], true, &mut counts);
        count.hits("b.rs", &[hit(1, "d", &[], false)], true, &mut counts);
        assert_eq!(String::from_utf8(counts).unwrap(), "a.rs:3\nb.rs:1\n");
        let mut files = Vec::new();
        Records::new(GrepOutput::Files, false, false, false).hits(
            "sub/c.rs",
            &[hit(7, "x", &[], false)],
            true,
            &mut files,
        );
        assert_eq!(String::from_utf8(files).unwrap(), "sub/c.rs\n");
        let summary = ContentSummary {
            files_searched: 12,
            files_matched: 2,
            hits: 5,
            skipped: 1,
            ..Default::default()
        };
        assert_eq!(
            totals_line("foo", &summary),
            "Suche „foo“ (5 Treffer in 2 Dateien von 12 durchsuchten; 1 übersprungen)"
        );
    }
}
