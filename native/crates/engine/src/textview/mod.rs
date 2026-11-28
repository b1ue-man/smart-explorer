//! Text view core (plan docs/plaene/2026-11-27-dateien-suche-ansicht, part D;
//! surfaces: docs/oberflaechen/textansicht.md). A [`TextDocument`] reads text
//! files of any size, local or remote, through a [`TextSource`]; the
//! [`Highlighter`] turns windows of lines into [`StyledLine`]s and is the only
//! part that knows languages; language packs are Sublime syntax files managed
//! by [`LanguageStore`]; Markdown becomes an [`MdDocument`] the surfaces draw
//! themselves. Binary and Office files are refused ([`TextViewError::NotText`]).
#[path = "core/builtin_licenses.rs"]
mod builtin_licenses;
#[path = "core/document.rs"]
mod document;
#[path = "core/encoding.rs"]
mod encoding;
#[path = "core/error.rs"]
mod error;
#[path = "core/highlight.rs"]
mod highlight;
#[path = "core/line_scan.rs"]
mod line_scan;
#[path = "core/markdown.rs"]
mod markdown;
#[path = "core/markdown_model.rs"]
mod markdown_model;
#[path = "core/pack_cache.rs"]
mod pack_cache;
#[path = "core/pack_files.rs"]
mod pack_files;
#[path = "core/packs.rs"]
mod packs;
#[path = "core/session.rs"]
mod session;
#[path = "core/source.rs"]
mod source;
#[path = "core/style.rs"]
mod style;

pub(crate) mod os;

pub use document::{
    IndexStatus, LineEndingCounts, LineProvider, TextDocument, TextLine, LINE_STRIDE,
    MAX_LINE_BYTES,
};
pub use encoding::{detect_encoding, TextEncoding, SNIFF_LEN};
pub use error::{TextViewError, TextViewResult};
pub use highlight::{
    shared_highlighter, Highlighter, LanguageInfo, DARK_THEME, LIGHT_THEME, MAX_HIGHLIGHT_CHARS,
    PLAIN_TEXT,
};
pub use line_scan::LineEnding;
pub use markdown::{parse_markdown, MAX_NESTING};
pub use markdown_model::{
    MdAlignment, MdBlock, MdDocument, MdListItem, MdQuoteKind, MdRun, MdRunKind,
};
pub use os::{open_backend, open_local, FileSource, VfsSource};
pub use pack_files::SYNTAX_EXTENSION;
pub use packs::{LanguagePack, LanguageStore, PackOrigin, PackSource};
pub use session::HighlightSession;
pub use source::{BytesSource, TextSource};
pub use style::{StyledLine, StyledSpan, ThemeColors, ThemeKind, TokenStyle};

#[cfg(test)]
#[path = "tests/document_tests.rs"]
mod document_tests;
#[cfg(test)]
#[path = "tests/highlight_tests.rs"]
mod highlight_tests;
#[cfg(test)]
#[path = "tests/markdown_tests.rs"]
mod markdown_tests;
#[cfg(test)]
#[path = "tests/pack_tests.rs"]
mod pack_tests;
