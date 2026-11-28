//! Content search ("Suchen in Dateien", plan docs/plaene/2026-11-27-dateien-suche-ansicht
//! section B): the request, the hits and the totals, shared by every place that
//! searches — the local search of the app, the SSH agent (`se-agent` includes this
//! module), the Share host (`FsRequest::ContentSearch` mirrors these fields) and the
//! search that reads files through any backend.
//!
//! Plain data without serde: the agent protocol encodes it with its own codec, the
//! Share wire has serde mirrors in `engine::share`.

/// How `ContentQuery::pattern` is read, as Notepad++'s "Search Mode".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextMode {
    /// The text itself.
    #[default]
    Normal,
    /// The text with Notepad++'s escapes `\n`, `\r`, `\t`, `\0`, `\\`, `\xHH`, `\uHHHH`
    /// (and the further ones Notepad++ documents) turned into characters.
    Extended,
    /// A regular expression: Rust `regex` syntax, plus the Boost/Perl features
    /// Notepad++ users write (`\R`, `\<`, `\>`, lookaround, back references, atomic
    /// groups) through a backtracking matcher where `regex` lacks them; `^`/`$`
    /// match at line ends and a pattern that can match a line end searches across
    /// lines, as Notepad++'s regular expressions do.
    Regex,
}

impl TextMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Extended => "extended",
            Self::Regex => "regex",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "normal" | "text" | "literal" => Some(Self::Normal),
            "extended" => Some(Self::Extended),
            "regex" | "regexp" => Some(Self::Regex),
            _ => None,
        }
    }
}

/// One content search below a root folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentQuery {
    /// What to find; never empty.
    pub pattern: String,
    pub mode: TextMode,
    /// Notepad++ "Match case"; false = case-insensitive (Unicode simple folding).
    pub match_case: bool,
    /// Notepad++ "Match whole word only" (Normal and Extended; Notepad++ disables it
    /// for regular expressions, so it is ignored there).
    pub whole_word: bool,
    /// Notepad++ ". matches newline" (only with `TextMode::Regex`); a pattern that
    /// can match a line end searches across lines.
    pub dot_matches_newline: bool,
    /// Notepad++ "Filters" field exactly (plan section B1): patterns separated by
    /// white space, no quoting; `*` and `?` wildcards matched case-insensitively
    /// against the file name only (DOS semantics: `*.*` = every name); `!pattern`
    /// excludes files and wins over every inclusion; `!\pattern` excludes folders of
    /// the first level below the root by name, `!+\pattern` folders of every level.
    /// Empty, or only exclusions, = `*.*` plus the exclusions.
    pub filters: String,
    /// Regex on the folder path relative to the root, `/`-separated, without leading
    /// or trailing `/` (the root itself is ""): files are searched only in folders
    /// whose path matches. None = every folder. (User 2026-11-27: "ordnerpfad mit regex".)
    pub folder_include: Option<String>,
    /// Regex on the same folder path: a matching folder and everything below it is
    /// skipped (not entered).
    pub folder_exclude: Option<String>,
    /// Notepad++ "In all sub-folders"; false = only the files directly in the root.
    pub subfolders: bool,
    /// Notepad++ "In hidden folders": also enter hidden folders (Windows: the hidden
    /// attribute; elsewhere a name starting with `.`). Hidden files are always
    /// searched, as in Notepad++.
    pub hidden: bool,
    /// Lines before and after each hit (`grep -C`); 0 = none.
    pub context_lines: u32,
    /// Stop after this many hit lines in total; 0 = unlimited.
    pub max_hits: u64,
    /// Only name the files that contain a hit (the search of a file ends at its
    /// first hit); `ContentEvent::Hits` then carries one hit per file.
    pub files_only: bool,
    /// Longest line text a hit carries, in bytes; a longer line is cut around the
    /// match and marked `truncated`. 0 = `DEFAULT_LINE_BYTES`.
    pub max_line_bytes: u32,
    /// Skip what the folder's ignore files exclude, as ripgrep does: `.gitignore`
    /// (inside a Git work tree), `.ignore`, `.git/info/exclude` (plan
    /// docs/plaene/2026-11-28-ersetzen-masken-shell, part C). false = Notepad++'s
    /// behaviour (every file the filters admit). An SSH agent before protocol 14 and
    /// a Share host without `content_search_ignore_v1` never receive it; the
    /// search then reads through the backend and filters itself.
    pub respect_ignore_files: bool,
}

/// Line text a hit carries when `ContentQuery::max_line_bytes` is 0.
pub const DEFAULT_LINE_BYTES: u32 = 1024;

impl ContentQuery {
    /// A literal, case-insensitive search of every file below the root.
    pub fn text(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            mode: TextMode::Normal,
            match_case: false,
            whole_word: false,
            dot_matches_newline: false,
            filters: String::new(),
            folder_include: None,
            folder_exclude: None,
            subfolders: true,
            hidden: false,
            context_lines: 0,
            max_hits: 0,
            files_only: false,
            max_line_bytes: 0,
            respect_ignore_files: false,
        }
    }

    pub fn line_bytes(&self) -> usize {
        match self.max_line_bytes {
            0 => DEFAULT_LINE_BYTES as usize,
            bytes => bytes as usize,
        }
    }
}

/// One line of a file: a hit, or a context line around hits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHit {
    /// 1-based line number.
    pub line: u64,
    /// Byte offset of the line's first byte in the file (for a UTF-16 file, or
    /// an ANSI file searched transcoded from Windows-1252, in its UTF-8 text).
    pub offset: u64,
    /// The line without its line end, decoded as UTF-8; a line that is not
    /// valid UTF-8 is decoded as Windows-1252 (as Notepad++ shows an ANSI
    /// file). Cut to `max_line_bytes` (at most 1 MiB) around the first match.
    pub text: String,
    /// Byte ranges of the matches within `text` (start, end); empty for context lines.
    pub ranges: Vec<(u32, u32)>,
    /// `text` is a cut of a longer line.
    pub truncated: bool,
    /// A context line (`context_lines`), not a hit.
    pub context: bool,
}

/// Why a file below the root was not searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// Contains NUL bytes (not a text file; binary search is out of scope).
    Binary,
    /// Could not be opened or read; the text names the cause.
    Unreadable(String),
    /// A folder that could not be listed; the text names the cause.
    UnlistableFolder(String),
}

/// What a running content search reports, in order per file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentEvent {
    /// Lines of one file (`rel` relative to the root, `/`-separated). A file with
    /// many hits comes in several events; `last` marks its final one.
    Hits {
        rel: String,
        size: u64,
        mtime_ms: i64,
        lines: Vec<LineHit>,
        last: bool,
    },
    /// A file or folder that was not searched.
    Skipped { rel: String, reason: SkipReason },
    /// Running totals, at most a few times per second.
    Progress { files: u64, bytes: u64, hits: u64 },
}

/// The totals of a finished search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentSummary {
    /// Files whose content was searched.
    pub files_searched: u64,
    /// Files with at least one hit.
    pub files_matched: u64,
    /// Hit lines (context lines not counted).
    pub hits: u64,
    /// Bytes read.
    pub bytes: u64,
    /// Files and folders reported as `Skipped`.
    pub skipped: u64,
    /// The search ended at `max_hits`.
    pub limited: bool,
}
