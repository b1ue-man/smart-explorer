//! Wire form of a content search on the host (`content_search_v1`,
//! `FsRequest::ContentSearch`, `FsResponse::ContentSearch`): serde mirrors
//! of `agent_proto::ContentQuery`, `ContentEvent` and `ContentSummary`
//! (plan docs/plaene/2026-11-27-dateien-suche-ansicht, section B). Every
//! field is additive; values of later versions read as the nearest known
//! one (`Unknown` search modes are refused, unknown skip reasons read as
//! `Unreadable`).
use serde::{Deserialize, Serialize};
use std::io;

use crate::agent_proto::{
    ContentEvent, ContentQuery, ContentSummary, LineHit, SkipReason, TextMode,
};

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn yes() -> bool {
    true
}

/// Longest line text a hit carries on the wire: a host cuts longer lines
/// to it whatever the request asks, so one line always fits a portion.
pub const CONTENT_LINE_BYTES_MAX: u32 = 64 * 1024;

/// `content_search_v1`: search the file contents below `path` on the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsContentSearch {
    pub path: String,
    pub query: FsContentQuery,
}

/// How the pattern is read (`agent_proto::TextMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsTextMode {
    #[default]
    Normal,
    Extended,
    Regex,
    /// A mode of a later client; the host answers `Unsupported`.
    #[serde(other)]
    Unknown,
}

/// `agent_proto::ContentQuery` on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsContentQuery {
    pub pattern: String,
    #[serde(default)]
    pub mode: FsTextMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub match_case: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub whole_word: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dot_matches_newline: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filters: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_include: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_exclude: Option<String>,
    #[serde(default = "yes")]
    pub subfolders: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub context_lines: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub max_hits: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub files_only: bool,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub max_line_bytes: u32,
    /// `ContentQuery::respect_ignore_files`; sent only to a host that offers
    /// `content_search_ignore_v1` (an older host would drop the field and
    /// search every file).
    #[serde(default, skip_serializing_if = "is_false")]
    pub respect_ignore_files: bool,
}

impl From<&ContentQuery> for FsContentQuery {
    fn from(query: &ContentQuery) -> Self {
        Self {
            pattern: query.pattern.clone(),
            mode: match query.mode {
                TextMode::Normal => FsTextMode::Normal,
                TextMode::Extended => FsTextMode::Extended,
                TextMode::Regex => FsTextMode::Regex,
            },
            match_case: query.match_case,
            whole_word: query.whole_word,
            dot_matches_newline: query.dot_matches_newline,
            filters: query.filters.clone(),
            folder_include: query.folder_include.clone(),
            folder_exclude: query.folder_exclude.clone(),
            subfolders: query.subfolders,
            hidden: query.hidden,
            context_lines: query.context_lines,
            max_hits: query.max_hits,
            files_only: query.files_only,
            max_line_bytes: query.max_line_bytes,
            respect_ignore_files: query.respect_ignore_files,
        }
    }
}

impl FsContentQuery {
    /// The query the host runs: an unknown mode or an empty pattern is
    /// refused; the line text is cut to `CONTENT_LINE_BYTES_MAX`.
    pub fn to_query(&self) -> io::Result<ContentQuery> {
        let mode = match self.mode {
            FsTextMode::Normal => TextMode::Normal,
            FsTextMode::Extended => TextMode::Extended,
            FsTextMode::Regex => TextMode::Regex,
            FsTextMode::Unknown => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Suchmodus nicht unterstützt",
                ))
            }
        };
        if self.pattern.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Suchtext fehlt",
            ));
        }
        let mut query = ContentQuery::text(self.pattern.clone());
        query.mode = mode;
        query.match_case = self.match_case;
        query.whole_word = self.whole_word;
        query.dot_matches_newline = self.dot_matches_newline;
        query.filters = self.filters.clone();
        query.folder_include = self.folder_include.clone();
        query.folder_exclude = self.folder_exclude.clone();
        query.subfolders = self.subfolders;
        query.hidden = self.hidden;
        query.context_lines = self.context_lines;
        query.max_hits = self.max_hits;
        query.files_only = self.files_only;
        query.max_line_bytes = match self.max_line_bytes {
            0 => 0,
            bytes => bytes.min(CONTENT_LINE_BYTES_MAX),
        };
        query.respect_ignore_files = self.respect_ignore_files;
        Ok(query)
    }
}

/// The stream of one content search: hits and skips in portions, running
/// totals while the host reads, and the closing totals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "m", rename_all = "snake_case")]
pub enum FsContentMessage {
    Progress {
        files: u64,
        bytes: u64,
        hits: u64,
    },
    Events {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<FsContentEvent>,
    },
    /// The search is complete; `hits` and `skipped` equal what the portions
    /// carried.
    Done {
        summary: FsContentSummary,
    },
}

/// One `agent_proto::ContentEvent` other than `Progress`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum FsContentEvent {
    Hits {
        rel: String,
        #[serde(default)]
        size: u64,
        #[serde(default)]
        mtime_ms: i64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lines: Vec<FsLineHit>,
        #[serde(default, skip_serializing_if = "is_false")]
        last: bool,
    },
    Skipped {
        rel: String,
        reason: FsSkipKind,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        detail: String,
    },
}

/// Why a file or folder was not searched (`agent_proto::SkipReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsSkipKind {
    Binary,
    UnlistableFolder,
    #[serde(other)]
    Unreadable,
}

/// `agent_proto::LineHit` on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsLineHit {
    pub line: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<(u32, u32)>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub context: bool,
}

/// `agent_proto::ContentSummary` on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsContentSummary {
    #[serde(default)]
    pub files_searched: u64,
    #[serde(default)]
    pub files_matched: u64,
    #[serde(default)]
    pub hits: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub skipped: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub limited: bool,
}

impl FsLineHit {
    /// JSON bytes this line adds to a portion, with every text byte escaped
    /// at the worst (`\u00XX`).
    pub fn wire_bytes(&self) -> usize {
        self.text
            .len()
            .saturating_mul(6)
            .saturating_add(self.ranges.len() * 24 + 96)
    }

    /// Every match range lies within the text, on its character bounds.
    pub fn well_formed(&self) -> bool {
        self.ranges.iter().all(|&(start, end)| {
            start <= end
                && self.text.is_char_boundary(start as usize)
                && self.text.is_char_boundary(end as usize)
        })
    }
}

impl FsContentEvent {
    /// The wire form of `event`; `None` for `Progress` (the stream carries
    /// its own totals).
    pub fn from_event(event: ContentEvent) -> Option<Self> {
        match event {
            ContentEvent::Hits {
                rel,
                size,
                mtime_ms,
                lines,
                last,
            } => Some(Self::Hits {
                rel,
                size,
                mtime_ms,
                lines: lines.into_iter().map(Into::into).collect(),
                last,
            }),
            ContentEvent::Skipped { rel, reason } => {
                let (reason, detail) = match reason {
                    SkipReason::Binary => (FsSkipKind::Binary, String::new()),
                    SkipReason::Unreadable(detail) => (FsSkipKind::Unreadable, detail),
                    SkipReason::UnlistableFolder(detail) => (FsSkipKind::UnlistableFolder, detail),
                };
                Some(Self::Skipped {
                    rel,
                    reason,
                    detail,
                })
            }
            ContentEvent::Progress { .. } => None,
        }
    }

    pub fn rel(&self) -> &str {
        match self {
            Self::Hits { rel, .. } | Self::Skipped { rel, .. } => rel,
        }
    }

    /// JSON bytes this event adds to a portion (estimate, see
    /// `FsLineHit::wire_bytes`).
    pub fn wire_bytes(&self) -> usize {
        let rel = self.rel().len().saturating_mul(6).saturating_add(96);
        match self {
            Self::Hits { lines, .. } => lines
                .iter()
                .map(FsLineHit::wire_bytes)
                .fold(rel, usize::saturating_add),
            Self::Skipped { detail, .. } => rel.saturating_add(detail.len().saturating_mul(6)),
        }
    }
}

impl From<LineHit> for FsLineHit {
    fn from(hit: LineHit) -> Self {
        Self {
            line: hit.line,
            offset: hit.offset,
            text: hit.text,
            ranges: hit.ranges,
            truncated: hit.truncated,
            context: hit.context,
        }
    }
}

impl From<FsLineHit> for LineHit {
    fn from(hit: FsLineHit) -> Self {
        Self {
            line: hit.line,
            offset: hit.offset,
            text: hit.text,
            ranges: hit.ranges,
            truncated: hit.truncated,
            context: hit.context,
        }
    }
}

impl From<FsContentEvent> for ContentEvent {
    fn from(event: FsContentEvent) -> Self {
        match event {
            FsContentEvent::Hits {
                rel,
                size,
                mtime_ms,
                lines,
                last,
            } => Self::Hits {
                rel,
                size,
                mtime_ms,
                lines: lines.into_iter().map(Into::into).collect(),
                last,
            },
            FsContentEvent::Skipped {
                rel,
                reason,
                detail,
            } => Self::Skipped {
                rel,
                reason: match reason {
                    FsSkipKind::Binary => SkipReason::Binary,
                    FsSkipKind::UnlistableFolder => SkipReason::UnlistableFolder(detail),
                    FsSkipKind::Unreadable => SkipReason::Unreadable(detail),
                },
            },
        }
    }
}

impl From<&ContentSummary> for FsContentSummary {
    fn from(summary: &ContentSummary) -> Self {
        Self {
            files_searched: summary.files_searched,
            files_matched: summary.files_matched,
            hits: summary.hits,
            bytes: summary.bytes,
            skipped: summary.skipped,
            limited: summary.limited,
        }
    }
}

impl From<FsContentSummary> for ContentSummary {
    fn from(summary: FsContentSummary) -> Self {
        Self {
            files_searched: summary.files_searched,
            files_matched: summary.files_matched,
            hits: summary.hits,
            bytes: summary.bytes,
            skipped: summary.skipped,
            limited: summary.limited,
        }
    }
}

#[cfg(test)]
#[path = "content_search_wire_task_tests.rs"]
mod task_tests;
