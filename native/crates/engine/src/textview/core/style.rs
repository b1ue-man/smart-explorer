//! What a view paints: lines of text with colored spans. The view knows no
//! languages and no grammar; it only receives these values from the
//! [`Highlighter`](super::highlight::Highlighter). Colors are RGBA bytes.
use serde::{Deserialize, Serialize};

/// Look of one span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenStyle {
    /// Text color, RGBA.
    pub fg: [u8; 4],
    /// Background color, RGBA; `None` = the view's background.
    pub bg: Option<[u8; 4]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// A styled range of a line, in byte offsets of `StyledLine::text`
/// (always on UTF-8 character boundaries).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StyledSpan {
    pub start: usize,
    pub end: usize,
    pub style: TokenStyle,
}

/// One line with its spans. The spans cover the text without gaps, in order;
/// an empty line has no spans.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StyledLine {
    pub text: String,
    pub spans: Vec<StyledSpan>,
}

impl StyledLine {
    /// The whole line in one style.
    pub fn plain(text: String, style: TokenStyle) -> Self {
        let spans = if text.is_empty() {
            Vec::new()
        } else {
            vec![StyledSpan {
                start: 0,
                end: text.len(),
                style,
            }]
        };
        Self { text, spans }
    }
}

/// Light or dark color theme of the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeKind {
    #[default]
    Light,
    Dark,
}

impl ThemeKind {
    /// Parses "light"/"hell" and "dark"/"dunkel" (any case).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "light" | "hell" => Some(Self::Light),
            "dark" | "dunkel" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Colors of the view around the text, taken from the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ThemeColors {
    /// Name of the syntect theme.
    pub name: &'static str,
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub selection: [u8; 4],
    pub line_highlight: [u8; 4],
    pub gutter_foreground: [u8; 4],
}
