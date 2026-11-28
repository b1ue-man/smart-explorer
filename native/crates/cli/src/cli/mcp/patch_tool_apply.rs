//! The new content of an updated file, computed as Codex computes it
//! (openai/codex `rust-v0.166.0`, `codex-rs/apply-patch/src/file_update.rs`,
//! `seek_sequence.rs`, `text_file.rs`): each `@@` section is found after the
//! previous one, first by its context line, then its old lines with
//! decreasing strictness (exact, without trailing white space, trimmed, with
//! typographic dashes, quotes and spaces read as ASCII); a section marked
//! `*** End of File` is looked for at the end first. Unchanged lines keep
//! their own line ends, inserted lines get the file's first line end (LF in
//! a file without one), and every line of the result ends with a line end.

/// `(start index, old line count, new lines)`.
type Replacement = (usize, usize, Vec<String>);

use super::patch_tool_parse::Chunk;

/// Typographic punctuation and spaces as their ASCII forms (Codex's last,
/// most permissive pass).
fn normalise(text: &str) -> String {
    text.trim()
        .chars()
        .map(|character| match character {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
            | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
            | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// The first index at or after `start` where `pattern` occurs in `lines`.
pub(super) fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let search_start = if eof {
        (lines.len() - pattern.len()).max(start)
    } else {
        start
    };
    let last = lines.len() - pattern.len();
    let passes: [&dyn Fn(&str, &str) -> bool; 4] = [
        &|line, wanted| line == wanted,
        &|line, wanted| line.trim_end() == wanted.trim_end(),
        &|line, wanted| line.trim() == wanted.trim(),
        &|line, wanted| normalise(line) == normalise(wanted),
    ];
    for same in passes {
        for index in search_start..=last {
            if pattern
                .iter()
                .enumerate()
                .all(|(offset, wanted)| same(&lines[index + offset], wanted))
            {
                return Some(index);
            }
        }
    }
    None
}

fn replacements(
    lines: &[String],
    path: &str,
    chunks: &[Chunk],
) -> Result<Vec<Replacement>, String> {
    let mut out: Vec<Replacement> = Vec::new();
    let mut line_index = 0usize;
    for chunk in chunks {
        if let Some(context) = &chunk.change_context {
            match seek_sequence(lines, std::slice::from_ref(context), line_index, false) {
                Some(index) => line_index = index + 1,
                None => return Err(format!("Failed to find context '{context}' in {path}")),
            }
        }
        if chunk.old_lines.is_empty() {
            out.push((lines.len(), 0, chunk.new_lines.clone()));
            continue;
        }
        let mut pattern: &[String] = &chunk.old_lines;
        let mut new_slice: &[String] = &chunk.new_lines;
        let mut found = seek_sequence(lines, pattern, line_index, chunk.is_end_of_file);
        // A trailing empty line stands for the final line end of the file.
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(lines, pattern, line_index, chunk.is_end_of_file);
        }
        let Some(start) = found else {
            return Err(format!(
                "Failed to find expected lines in {path}:\n{}",
                chunk.old_lines.join("\n")
            ));
        };
        // Context lines stay as they are in the file (their text and ends).
        let (mut old_start, mut new_start) = (0usize, 0usize);
        for &(old_context, new_context) in &chunk.context_line_indices {
            if old_context >= pattern.len() || new_context >= new_slice.len() {
                break;
            }
            if old_start != old_context || new_start != new_context {
                out.push((
                    start + old_start,
                    old_context - old_start,
                    new_slice[new_start..new_context].to_vec(),
                ));
            }
            old_start = old_context + 1;
            new_start = new_context + 1;
        }
        if old_start != pattern.len() || new_start != new_slice.len() {
            out.push((
                start + old_start,
                pattern.len() - old_start,
                new_slice[new_start..].to_vec(),
            ));
        }
        line_index = start + pattern.len();
    }
    out.sort_by_key(|(index, _, _)| *index);
    Ok(out)
}

/// One line of a file with its line end ("" for the last line without one).
struct SourceLine {
    text: String,
    ending: Option<&'static str>,
}

fn parse_lines(contents: &str) -> (Vec<SourceLine>, &'static str) {
    let bytes = contents.as_bytes();
    let mut lines = Vec::new();
    let mut preferred = None;
    let (mut start, mut cursor) = (0usize, 0usize);
    while cursor < bytes.len() {
        let (ending, length) = match bytes[cursor] {
            b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => ("\r\n", 2),
            b'\r' => ("\r", 1),
            b'\n' => ("\n", 1),
            _ => {
                cursor += 1;
                continue;
            }
        };
        preferred.get_or_insert(ending);
        lines.push(SourceLine {
            text: contents[start..cursor].to_string(),
            ending: Some(ending),
        });
        cursor += length;
        start = cursor;
    }
    if start < contents.len() {
        lines.push(SourceLine {
            text: contents[start..].to_string(),
            ending: None,
        });
    }
    (lines, preferred.unwrap_or("\n"))
}

/// `original` with the `chunks` of an update applied (`path` names it in
/// errors).
pub(super) fn updated_contents(
    original: &str,
    path: &str,
    chunks: &[Chunk],
) -> Result<String, String> {
    let (lines, preferred) = parse_lines(original);
    let texts: Vec<String> = lines.iter().map(|line| line.text.clone()).collect();
    let replacements = replacements(&texts, path, chunks)?;
    let mut source = lines.into_iter();
    let mut result: Vec<SourceLine> = Vec::new();
    let mut index = 0usize;
    for (start, old_len, new_lines) in replacements {
        // Overlapping sections (a later one found before the end of an
        // earlier one) cannot both apply; Codex asserts this in debug builds.
        if start < index {
            return Err(format!(
                "Failed to apply the patch to {path}: two sections change the same lines"
            ));
        }
        result.extend(source.by_ref().take(start - index));
        for _ in source.by_ref().take(old_len) {}
        result.extend(new_lines.into_iter().map(|text| SourceLine {
            text,
            ending: Some(preferred),
        }));
        index = start + old_len;
    }
    result.extend(source);
    let mut contents = String::with_capacity(original.len());
    for line in result {
        contents.push_str(&line.text);
        contents.push_str(line.ending.unwrap_or(preferred));
    }
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::super::patch_tool_parse::{parse_patch, Hunk};
    use super::{seek_sequence, updated_contents};

    fn update(original: &str, patch: &str) -> Result<String, String> {
        let hunks = parse_patch(patch).unwrap();
        let Hunk::Update { chunks, .. } = &hunks[0] else {
            panic!("{hunks:?}");
        };
        updated_contents(original, "f", chunks)
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn mcp_coding_task_patch_updates_like_codex() {
        // Exact, trailing-space, trimmed and typographic matches.
        let file = lines(&["foo   ", "  bar\t", "a \u{2013} b"]);
        assert_eq!(seek_sequence(&file, &lines(&["foo"]), 0, false), Some(0));
        assert_eq!(seek_sequence(&file, &lines(&["bar"]), 0, false), Some(1));
        assert_eq!(seek_sequence(&file, &lines(&["a - b"]), 0, false), Some(2));
        assert_eq!(
            seek_sequence(&file, &lines(&["x", "y", "z", "w"]), 0, false),
            None
        );

        let patch =
            "*** Begin Patch\n*** Update File: f\n@@ fn two\n-    2\n+    22\n*** End Patch";
        assert_eq!(
            update("fn one\n    2\nfn two\n    2\n", patch).unwrap(),
            "fn one\n    2\nfn two\n    22\n",
            "the context line moves the section to the second definition"
        );
        // Line ends of unchanged lines stay; inserted lines take the first.
        let crlf = "*** Begin Patch\n*** Update File: f\n@@\n a\n-b\n+B\n+C\n*** End Patch";
        assert_eq!(update("a\r\nb\r\nc\n", crlf).unwrap(), "a\r\nB\r\nC\r\nc\n");
        // A last line without line end gets one.
        let tail =
            "*** Begin Patch\n*** Update File: f\n@@\n-x\n+y\n*** End of File\n*** End Patch";
        assert_eq!(update("x\nx", tail).unwrap(), "x\ny\n");
        // A pure insertion goes to the end of the file.
        let append = "*** Begin Patch\n*** Update File: f\n@@\n+z\n*** End Patch";
        assert_eq!(update("a\n", append).unwrap(), "a\nz\n");
        let missing = "*** Begin Patch\n*** Update File: f\n@@\n-nope\n+x\n*** End Patch";
        assert_eq!(
            update("a\n", missing),
            Err("Failed to find expected lines in f:\nnope".to_string())
        );
        let context = "*** Begin Patch\n*** Update File: f\n@@ nowhere\n-a\n+b\n*** End Patch";
        assert_eq!(
            update("a\n", context),
            Err("Failed to find context 'nowhere' in f".to_string())
        );
    }
}
