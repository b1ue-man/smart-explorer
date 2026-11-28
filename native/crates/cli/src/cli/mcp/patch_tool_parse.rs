//! The patch format of Codex's `apply_patch` (openai/codex, tag
//! `rust-v0.166.0`, `codex-rs/apply-patch/src/parser.rs` and
//! `streaming_parser.rs`), parsed as Codex parses it:
//!
//! ```text
//! start: begin_patch environment_id? hunk+ end_patch
//! begin_patch: "*** Begin Patch" LF
//! environment_id: "*** Environment ID: " filename LF
//! end_patch: "*** End Patch" LF?
//! hunk: add_hunk | delete_hunk | update_hunk
//! add_hunk: "*** Add File: " filename LF add_line+
//! delete_hunk: "*** Delete File: " filename LF
//! update_hunk: "*** Update File: " filename LF change_move? change?
//! add_line: "+" /(.+)/ LF
//! change_move: "*** Move to: " filename LF
//! change: (change_context | change_line)+ eof_line?
//! change_context: ("@@" | "@@ " /(.+)/) LF
//! change_line: ("+" | "-" | " ") /(.+)/ LF
//! eof_line: "*** End of File" LF
//! ```
//!
//! Like Codex it is lenient about white space around the markers and about a
//! patch wrapped in a heredoc (`<<'EOF'` … `EOF`). Error texts are Codex's.
//! The environment id is read and has no effect here (se resolves every
//! path against the working root or `target`).

pub(super) const BEGIN_PATCH: &str = "*** Begin Patch";
pub(super) const END_PATCH: &str = "*** End Patch";
const ADD_FILE: &str = "*** Add File: ";
const DELETE_FILE: &str = "*** Delete File: ";
const UPDATE_FILE: &str = "*** Update File: ";
const MOVE_TO: &str = "*** Move to: ";
const END_OF_FILE: &str = "*** End of File";
const CHANGE_CONTEXT: &str = "@@ ";
const EMPTY_CHANGE_CONTEXT: &str = "@@";
const ENVIRONMENT_ID: &str = "*** Environment ID:";

/// What the patch does to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Hunk {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_path: Option<String>,
        chunks: Vec<Chunk>,
    },
}

impl Hunk {
    pub(super) fn path(&self) -> &str {
        match self {
            Self::Add { path, .. } | Self::Delete { path } | Self::Update { path, .. } => path,
        }
    }
}

/// One `@@` section of an update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Chunk {
    /// A line (usually a definition) that occurs before `old_lines`.
    pub(super) change_context: Option<String>,
    pub(super) old_lines: Vec<String>,
    pub(super) new_lines: Vec<String>,
    /// Indices into `old_lines` and `new_lines` of the context lines (` `).
    pub(super) context_line_indices: Vec<(usize, usize)>,
    /// `old_lines` must end the file.
    pub(super) is_end_of_file: bool,
}

impl Chunk {
    fn push_context_line(&mut self, line: String) {
        self.context_line_indices
            .push((self.old_lines.len(), self.new_lines.len()));
        self.old_lines.push(line.clone());
        self.new_lines.push(line);
    }
}

#[derive(Clone, Copy)]
enum Mode {
    NotStarted,
    Started,
    Add,
    Delete,
    Update { header_line: usize },
    Ended,
}

struct Parser {
    mode: Mode,
    hunks: Vec<Hunk>,
    environment_id: Option<String>,
    line_number: usize,
}

const NOT_A_HEADER: &str = "is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'";

fn hunk_error(line_number: usize, message: String) -> String {
    format!("invalid hunk at line {line_number}, {message}")
}

fn patch_error(message: &str) -> String {
    format!("invalid patch: {message}")
}

fn unexpected(line: &str) -> String {
    format!(
        "Unexpected line found in update hunk: '{line}'. Every line should start with ' ' (context line), '+' (added line), or '-' (removed line)"
    )
}

impl Parser {
    fn ensure_update_not_empty(&self, line: &str) -> Result<(), String> {
        if let Some(Hunk::Update { path, chunks, .. }) = self.hunks.last() {
            if let (true, Mode::Update { header_line }) = (chunks.is_empty(), self.mode) {
                return Err(hunk_error(
                    header_line,
                    format!("Update file hunk for path '{path}' is empty"),
                ));
            }
            if chunks
                .last()
                .is_some_and(|chunk| chunk.old_lines.is_empty() && chunk.new_lines.is_empty())
            {
                let message = if line == END_PATCH {
                    "Update hunk does not contain any lines".to_string()
                } else {
                    unexpected(line)
                };
                return Err(hunk_error(self.line_number, message));
            }
        }
        Ok(())
    }

    /// A hunk header or the end; `Ok(true)` when `trimmed` was one.
    fn header(&mut self, trimmed: &str) -> Result<bool, String> {
        if let (Mode::Started, Some(id)) = (self.mode, trimmed.strip_prefix(ENVIRONMENT_ID)) {
            if self.environment_id.is_some() {
                return Err(patch_error(
                    "apply_patch environment_id cannot be specified more than once",
                ));
            }
            let id = id.trim();
            if id.is_empty() {
                return Err(patch_error("apply_patch environment_id cannot be empty"));
            }
            self.environment_id = Some(id.to_string());
            return Ok(true);
        }
        if trimmed == END_PATCH {
            self.ensure_update_not_empty(trimmed)?;
            self.mode = Mode::Ended;
            return Ok(true);
        }
        let (hunk, mode) = if let Some(path) = trimmed.strip_prefix(ADD_FILE) {
            let hunk = Hunk::Add {
                path: path.to_string(),
                contents: String::new(),
            };
            (hunk, Mode::Add)
        } else if let Some(path) = trimmed.strip_prefix(DELETE_FILE) {
            let hunk = Hunk::Delete {
                path: path.to_string(),
            };
            (hunk, Mode::Delete)
        } else if let Some(path) = trimmed.strip_prefix(UPDATE_FILE) {
            let hunk = Hunk::Update {
                path: path.to_string(),
                move_path: None,
                chunks: Vec::new(),
            };
            let header_line = self.line_number;
            (hunk, Mode::Update { header_line })
        } else {
            return Ok(false);
        };
        self.ensure_update_not_empty(trimmed)?;
        self.hunks.push(hunk);
        self.mode = mode;
        Ok(true)
    }

    fn line(&mut self, line: &str) -> Result<(), String> {
        let trimmed = line.trim();
        match self.mode {
            Mode::NotStarted => {
                if trimmed == BEGIN_PATCH {
                    self.mode = Mode::Started;
                    Ok(())
                } else {
                    Err(patch_error(
                        "The first line of the patch must be '*** Begin Patch'",
                    ))
                }
            }
            Mode::Started | Mode::Delete => {
                if self.header(trimmed)? {
                    return Ok(());
                }
                Err(hunk_error(
                    self.line_number,
                    format!("'{trimmed}' {NOT_A_HEADER}"),
                ))
            }
            Mode::Add => {
                if self.header(trimmed)? {
                    return Ok(());
                }
                if let (Some(added), Some(Hunk::Add { contents, .. })) =
                    (line.strip_prefix('+'), self.hunks.last_mut())
                {
                    contents.push_str(added);
                    contents.push('\n');
                    return Ok(());
                }
                Err(hunk_error(
                    self.line_number,
                    format!("'{trimmed}' {NOT_A_HEADER}"),
                ))
            }
            Mode::Update { .. } => self.update_line(line),
            Mode::Ended => {
                if trimmed.is_empty() {
                    Ok(())
                } else {
                    Err(patch_error(
                        "The last line of the patch must be '*** End Patch'",
                    ))
                }
            }
        }
    }

    fn update_line(&mut self, line: &str) -> Result<(), String> {
        let update_line = line.trim_end();
        if self.header(update_line)? {
            return Ok(());
        }
        let line_number = self.line_number;
        let Some(Hunk::Update {
            move_path, chunks, ..
        }) = self.hunks.last_mut()
        else {
            return Err(hunk_error(line_number, unexpected(line)));
        };
        let context_marker =
            update_line == EMPTY_CHANGE_CONTEXT || update_line.starts_with(CHANGE_CONTEXT);
        if chunks.last().is_some_and(|chunk| chunk.is_end_of_file) {
            if update_line.is_empty() {
                return Ok(());
            }
            if !context_marker {
                return Err(hunk_error(
                    line_number,
                    format!(
                        "Expected update hunk to start with a @@ context marker, got: '{line}'"
                    ),
                ));
            }
        }
        if chunks.is_empty() && move_path.is_none() {
            if let Some(to) = update_line.strip_prefix(MOVE_TO) {
                *move_path = Some(to.to_string());
                return Ok(());
            }
        }
        let last_empty = chunks
            .last()
            .is_some_and(|chunk| chunk.old_lines.is_empty() && chunk.new_lines.is_empty());
        if context_marker && last_empty {
            return Err(hunk_error(line_number, unexpected(line)));
        }
        if update_line == EMPTY_CHANGE_CONTEXT {
            chunks.push(Chunk::default());
            return Ok(());
        }
        if let Some(context) = update_line.strip_prefix(CHANGE_CONTEXT) {
            chunks.push(Chunk {
                change_context: Some(context.to_string()),
                ..Chunk::default()
            });
            return Ok(());
        }
        if update_line == END_OF_FILE {
            if last_empty {
                return Err(hunk_error(
                    line_number,
                    "Update hunk does not contain any lines".to_string(),
                ));
            }
            if let Some(chunk) = chunks.last_mut() {
                chunk.is_end_of_file = true;
            }
            return Ok(());
        }
        let current = |chunks: &mut Vec<Chunk>| {
            if chunks.is_empty() {
                chunks.push(Chunk::default());
            }
            chunks.len() - 1
        };
        if line.is_empty() {
            let index = current(chunks);
            chunks[index].push_context_line(String::new());
            return Ok(());
        }
        if let Some(context) = line.strip_prefix(' ') {
            let index = current(chunks);
            chunks[index].push_context_line(context.to_string());
            return Ok(());
        }
        if let Some(added) = line.strip_prefix('+') {
            let index = current(chunks);
            chunks[index].new_lines.push(added.to_string());
            return Ok(());
        }
        if let Some(removed) = line.strip_prefix('-') {
            let index = current(chunks);
            chunks[index].old_lines.push(removed.to_string());
            return Ok(());
        }
        if chunks
            .last()
            .is_some_and(|chunk| !chunk.old_lines.is_empty() || !chunk.new_lines.is_empty())
        {
            return Err(hunk_error(
                line_number,
                format!("Expected update hunk to start with a @@ context marker, got: '{line}'"),
            ));
        }
        Err(hunk_error(line_number, unexpected(line)))
    }
}

/// The first and last line must be the markers (after trimming); a patch in
/// a heredoc of at least four lines is unwrapped (Codex's lenient mode).
fn boundaries<'a>(lines: &'a [&'a str]) -> Result<&'a [&'a str], String> {
    let strict = |lines: &'a [&'a str]| -> Result<&'a [&'a str], String> {
        let first = lines.first().map(|line| line.trim());
        let last = lines.last().map(|line| line.trim());
        match (first, last) {
            (Some(first), Some(last)) if first == BEGIN_PATCH && last == END_PATCH => Ok(lines),
            (Some(first), _) if first != BEGIN_PATCH => Err(patch_error(
                "The first line of the patch must be '*** Begin Patch'",
            )),
            _ => Err(patch_error(
                "The last line of the patch must be '*** End Patch'",
            )),
        }
    };
    let error = match strict(lines) {
        Ok(lines) => return Ok(lines),
        Err(error) => error,
    };
    match lines {
        [first, .., last]
            if matches!(*first, "<<EOF" | "<<'EOF'" | "<<\"EOF\"")
                && last.ends_with("EOF")
                && lines.len() >= 4 =>
        {
            strict(&lines[1..lines.len() - 1])
        }
        _ => Err(error),
    }
}

/// The hunks of `patch`, in order.
pub(super) fn parse_patch(patch: &str) -> Result<Vec<Hunk>, String> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    let lines = boundaries(&lines)?;
    let mut parser = Parser {
        mode: Mode::NotStarted,
        hunks: Vec::new(),
        environment_id: None,
        line_number: 0,
    };
    let Some((last, rest)) = lines.split_last() else {
        return Err(patch_error(
            "The first line of the patch must be '*** Begin Patch'",
        ));
    };
    for line in rest {
        parser.line_number += 1;
        parser.line(line)?;
    }
    // The last line has no line end; Codex's `finish` takes the end marker
    // in any mode.
    parser.line_number += 1;
    if last.trim() == END_PATCH {
        parser.ensure_update_not_empty(last.trim())?;
        parser.mode = Mode::Ended;
    } else {
        parser.line(last)?;
    }
    if !matches!(parser.mode, Mode::Ended) {
        return Err(patch_error(
            "The last line of the patch must be '*** End Patch'",
        ));
    }
    Ok(parser.hunks)
}

#[cfg(test)]
#[path = "patch_tool_parse_tests.rs"]
mod tests;
