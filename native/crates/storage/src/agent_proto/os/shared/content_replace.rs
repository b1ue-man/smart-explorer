//! "Replace in Files" (plan docs/plaene/2026-11-28-ersetzen-masken-shell, part
//! C): the request, its events and totals, and the replacement in the content
//! of one file in memory. The matches are those of the content search
//! (`ContentMatcher`, across lines where the search runs across lines; empty
//! matches as Notepad++'s `SCFIND_REGEXP_EMPTYMATCH_NOTAFTERMATCH`: allowed,
//! but not directly after a match), the replacement text is read as Notepad++
//! reads it (Normal literal, Extended with its escapes, Regex with the Boost
//! `format_all` syntax, `content_replace_format.rs`), the encoding and the
//! line ends of the file stay (`content_replace_text.rs`). Plain data without
//! serde: this source is part of the se-agent as well. Reading and writing
//! the files is `vfs::content_replace`.
use std::borrow::Cow;

use super::content_extended::unescape_extended;
use super::content_match::ContentMatcher;
use super::content_query::{ContentQuery, TextMode};
use super::content_replace_captures::{Captor, Found};
use super::content_replace_format::{format, View};
use super::content_replace_out::Out;
use super::content_replace_select::{previews, selected, Lines};
use super::content_replace_text::{read, Haystack};

/// One "Replace in Files" below a root folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceRequest {
    /// The search whose matches are replaced (`files_only`, `max_hits` and
    /// `context_lines` do not apply: every match of a file is replaced).
    pub query: ContentQuery,
    /// The replacement, read in the mode of `query` (Normal: literal;
    /// Extended: Notepad++'s escapes; Regex: Boost format syntax).
    pub replacement: String,
    pub selection: ReplaceSelection,
    /// Only report what would change (`ReplaceEvent::Preview`), write nothing.
    pub dry_run: bool,
}

/// Which matches a replacement changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceSelection {
    /// Every match of every file the search finds.
    All,
    /// Every match in these files (relative to the root, `/`-separated).
    Files(Vec<String>),
    /// Only these hits of a search result; a file whose size or time differs
    /// from the hit's is skipped as changed since the search.
    Hits(Vec<HitRef>),
}

/// One hit of a search result, as `ContentEvent::Hits` reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitRef {
    /// The file, relative to the root.
    pub rel: String,
    /// 1-based line number (`LineHit::line`).
    pub line: u64,
    /// Ranges within the line's text (`LineHit::ranges`); empty = every
    /// match that touches the line.
    pub ranges: Vec<(u32, u32)>,
    /// Size and time of the file as the search listed it.
    pub size: u64,
    pub mtime_ms: i64,
}

/// Why a file was left unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceSkip {
    /// Contains NUL bytes (no text file).
    Binary,
    /// The file is write-protected.
    ReadOnly,
    /// The memory for the file could not be reserved.
    TooLarge,
    /// Could not be opened, read or decoded; the text names the cause.
    Unreadable(String),
    /// A folder the search could not list; the text names the cause.
    UnlistableFolder(String),
    /// The file changed since the search, or while it was replaced.
    Changed,
    /// An ANSI file and a replacement with characters Windows-1252 lacks.
    NotAnsi,
    /// Not a regular file (folder, link, special file) or not present.
    NotRegular,
    /// The encrypted copy of the original could not be kept; the file stays
    /// unchanged.
    BackupFailed(String),
    /// The new content could not be published; the file stays unchanged.
    WriteFailed(String),
}

impl ReplaceSkip {
    /// A stable key for machine output.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::ReadOnly => "read_only",
            Self::TooLarge => "too_large",
            Self::Unreadable(_) => "unreadable",
            Self::UnlistableFolder(_) => "unlistable_folder",
            Self::Changed => "changed",
            Self::NotAnsi => "not_ansi",
            Self::NotRegular => "not_regular",
            Self::BackupFailed(_) => "backup_failed",
            Self::WriteFailed(_) => "write_failed",
        }
    }

    /// The reason in German, for the user.
    pub fn message(&self) -> String {
        match self {
            Self::Binary => "keine Textdatei (enthält NUL-Bytes)".to_string(),
            Self::ReadOnly => "schreibgeschützt".to_string(),
            Self::TooLarge => "zu groß (Speicher nicht reservierbar)".to_string(),
            Self::Unreadable(cause) => format!("nicht lesbar: {cause}"),
            Self::UnlistableFolder(cause) => format!("Ordner nicht lesbar: {cause}"),
            Self::Changed => "seit der Suche geändert".to_string(),
            Self::NotAnsi => {
                "ANSI-Datei, der Ersatztext enthält Zeichen außerhalb von Windows-1252".to_string()
            }
            Self::NotRegular => "keine reguläre Datei oder nicht vorhanden".to_string(),
            Self::BackupFailed(cause) => {
                format!("Sicherung des Originals fehlgeschlagen, Datei unverändert: {cause}")
            }
            Self::WriteFailed(cause) => {
                format!("Schreiben fehlgeschlagen, Datei unverändert: {cause}")
            }
        }
    }
}

impl std::fmt::Display for ReplaceSkip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// What a running replacement reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceEvent {
    /// A file was changed (with `dry_run`: would be changed).
    File {
        rel: String,
        replacements: u64,
        size_before: u64,
        size_after: u64,
    },
    /// A file or folder left unchanged, with the reason.
    Skipped { rel: String, reason: ReplaceSkip },
    /// Running totals, at most a few times per second (the names of
    /// `ReplaceSummary`).
    Progress {
        files_searched: u64,
        files_changed: u64,
        replacements: u64,
    },
    /// `dry_run`: the lines of a change before and after (1-based `line` of
    /// the first; several lines where a match spans them).
    Preview {
        rel: String,
        line: u64,
        before: String,
        after: String,
        /// The texts are cuts of longer lines (`ContentQuery::line_bytes`).
        truncated: bool,
    },
}

/// The receiver of a replacement's events; `false` = it is gone, the run stops.
pub type ReplaceEmit<'a> = &'a (dyn Fn(ReplaceEvent) -> bool + Sync);

/// The totals of a finished replacement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceSummary {
    /// Files whose content was searched.
    pub files_searched: u64,
    /// Files changed (with `dry_run`: that would change).
    pub files_changed: u64,
    /// Replaced matches.
    pub replacements: u64,
    /// Files and folders reported as `Skipped`.
    pub skipped: u64,
    /// The run in the replace journal (`ReplaceJournal::undo`); `None` for a
    /// dry run or when nothing changed.
    pub undo_id: Option<String>,
    /// Ended early (cancel, receiver gone); files written before stay written.
    pub canceled: bool,
}

/// The lines of one change, as `ReplaceEvent::Preview` carries them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacePreview {
    pub line: u64,
    pub before: String,
    pub after: String,
    pub truncated: bool,
}

/// Which matches of one file are replaced.
#[derive(Clone, Copy, Debug)]
pub enum Pick<'a> {
    All,
    /// The hits of this file.
    Hits(&'a [HitRef]),
}

/// The new content of one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replaced {
    /// The whole new content, encoded as the file was (empty without
    /// replacements).
    pub content: Vec<u8>,
    pub replacements: u64,
    pub previews: Vec<ReplacePreview>,
}

/// How the replacement text is read.
#[derive(Debug)]
enum Form {
    /// Normal and Extended: the text (Extended after its escapes).
    Literal(String),
    /// Regex: Boost's `format_all` syntax.
    Boost(Vec<char>),
}

/// The compiled search and replacement of one run; shared by every thread.
#[derive(Debug)]
pub struct ReplaceProgram {
    matcher: ContentMatcher,
    captor: Captor,
    form: Form,
    line_bytes: usize,
}

impl ReplaceProgram {
    /// The program of `query` and `replacement`; the error (German) names an
    /// empty or invalid pattern.
    pub fn new(query: &ContentQuery, replacement: &str) -> Result<Self, String> {
        let matcher = ContentMatcher::new(query)?;
        let captor = Captor::new(&matcher, query)?;
        let form = match query.mode {
            TextMode::Normal => Form::Literal(replacement.to_string()),
            TextMode::Extended => Form::Literal(unescape_extended(replacement)),
            TextMode::Regex => Form::Boost(replacement.chars().collect()),
        };
        Ok(Self {
            matcher,
            captor,
            form,
            line_bytes: query.line_bytes(),
        })
    }

    /// Replaces the matches `pick` names in `content` (a whole file). `Err`
    /// for a file that stays as it is (binary, an encoding it cannot keep).
    pub fn apply(
        &self,
        content: &[u8],
        pick: Pick<'_>,
        with_previews: bool,
    ) -> Result<Replaced, ReplaceSkip> {
        let text = read(content)?;
        let mut haystack = text.haystack()?;
        let mut found = self.find(&haystack)?;
        if found.is_empty() && self.matcher.non_ascii() {
            // An ANSI file is searched transcoded as well (as the search does).
            if let Some(transcoded) = text.transcoded() {
                haystack = transcoded?;
                found = self.find(&haystack)?;
            }
        }
        if found.is_empty() {
            return Ok(Replaced::default());
        }
        let bytes: &[u8] = &haystack.bytes;
        let lines = match pick {
            Pick::Hits(_) => Some(Lines::new(bytes)),
            Pick::All if with_previews => Some(Lines::new(bytes)),
            Pick::All => None,
        };
        let chosen = match (pick, &lines) {
            (Pick::Hits(refs), Some(lines)) => {
                let shift = text.bom_shift();
                selected(bytes, lines, &found, refs, shift, self.line_bytes)
            }
            _ => vec![true; found.len()],
        };
        let mut out = Out::new(haystack.domain);
        out.bytes
            .try_reserve(bytes.len())
            .map_err(|_| ReplaceSkip::TooLarge)?;
        let names = |name: &str| self.captor.group_index(&self.matcher, name);
        let mut cursor = 0usize;
        let mut replacements = 0u64;
        // Output length minus input length after each chosen match.
        let mut shifts = Vec::new();
        for (item, _) in found.iter().zip(&chosen).filter(|(_, chosen)| **chosen) {
            let (start, end) = item.span();
            out.raw(&bytes[cursor..start]);
            out.begin_match();
            match &self.form {
                Form::Literal(text) => out.literal(text),
                Form::Boost(pattern) => {
                    let view = View {
                        haystack: bytes,
                        groups: &item.groups,
                        prefix_start: item.prefix_start,
                        names: &names,
                    };
                    format(pattern, &view, &mut out);
                }
            }
            cursor = end;
            replacements += 1;
            if with_previews {
                shifts.push(out.bytes.len() as i64 - end as i64);
            }
        }
        out.raw(&bytes[cursor..]);
        if out.unmappable {
            return Err(ReplaceSkip::NotAnsi);
        }
        if replacements == 0 {
            return Ok(Replaced::default());
        }
        let previews = match (&lines, with_previews) {
            (Some(lines), true) => previews(
                (bytes, &out.bytes),
                haystack.domain,
                lines,
                (&found, &chosen, &shifts),
                self.line_bytes,
            ),
            _ => Vec::new(),
        };
        let content = text.encode(&haystack, out.bytes)?;
        Ok(Replaced {
            content,
            replacements,
            previews,
        })
    }

    fn find(&self, haystack: &Haystack<'_>) -> Result<Vec<Found>, ReplaceSkip> {
        let bytes: &Cow<'_, [u8]> = &haystack.bytes;
        self.captor
            .find_all(&self.matcher, bytes)
            .map_err(|error| ReplaceSkip::Unreadable(error.to_string()))
    }
}
