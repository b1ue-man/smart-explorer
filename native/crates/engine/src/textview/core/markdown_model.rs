//! The rendering model of a Markdown document that every surface draws
//! itself (desktop with egui, Android with Compose; JSON over the bridge).
//! Blocks nest; text inside a block is a flat list of runs, each with its
//! own emphasis flags and link target.
use serde::{Deserialize, Serialize};

use super::style::StyledLine;

/// A parsed Markdown document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdDocument {
    pub blocks: Vec<MdBlock>,
}

/// One block of a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MdBlock {
    /// `#` to `######`; `level` is 1 to 6.
    Heading {
        level: u8,
        runs: Vec<MdRun>,
    },
    Paragraph {
        runs: Vec<MdRun>,
    },
    /// Ordered when `start` is set (the number of the first item); items
    /// may be task items and hold nested blocks (also nested lists).
    List {
        start: Option<u64>,
        items: Vec<MdListItem>,
    },
    /// `>` quote; `kind` for GitHub alerts (`> [!NOTE]`).
    Quote {
        kind: Option<MdQuoteKind>,
        blocks: Vec<MdBlock>,
    },
    /// Fenced or indented code. `language` is the fence's first word as
    /// written; `highlighted_as` the language used for `lines`.
    Code {
        language: Option<String>,
        highlighted_as: String,
        lines: Vec<StyledLine>,
    },
    /// Table: one alignment per column, the header cells and the rows.
    Table {
        alignments: Vec<MdAlignment>,
        header: Vec<Vec<MdRun>>,
        rows: Vec<Vec<Vec<MdRun>>>,
    },
    /// `---`
    Rule,
    /// A paragraph that is only one image.
    Image {
        url: String,
        title: String,
        alt: String,
    },
    /// Raw HTML block, shown as text (never interpreted).
    Html {
        text: String,
    },
    /// `[^label]: …`
    FootnoteDefinition {
        label: String,
        blocks: Vec<MdBlock>,
    },
}

/// An item of a list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdListItem {
    /// `Some(checked)` for a task item (`- [ ]`, `- [x]`).
    pub task: Option<bool>,
    pub blocks: Vec<MdBlock>,
}

/// Column alignment of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MdAlignment {
    None,
    Left,
    Center,
    Right,
}

/// Kind of a GitHub alert quote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MdQuoteKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// A run of inline content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdRun {
    /// The text (image: its alternative text; footnote reference: label).
    pub text: String,
    pub kind: MdRunKind,
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    /// Target of the link the run is part of.
    pub link: Option<String>,
}

/// What a run is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MdRunKind {
    #[default]
    Text,
    /// `` `code` ``
    Code,
    /// Hard line break (`text` is `"\n"`).
    LineBreak,
    /// Inline image; `text` is its alternative text.
    Image { url: String, title: String },
    /// `[^label]`
    FootnoteRef,
    /// Inline HTML, shown as text.
    Html,
}
