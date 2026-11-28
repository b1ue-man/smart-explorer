//! Syntax highlighting, separate from the view: picks the language of a file
//! (file name, extension, first line, or a name the user chose) and turns
//! lines into [`StyledLine`]s with the colors of the light or dark theme.
//! Languages come from Sublime syntax definitions (built in from two-face,
//! the set `bat` ships, plus the packs in the language folder); the view
//! itself never sees them.
use std::sync::{Arc, Mutex};

use serde::Serialize;
use syntect::highlighting::{Color, FontStyle, Style, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::EmbeddedThemeName;

use super::session::HighlightSession;
use super::style::{StyledLine, ThemeColors, ThemeKind, TokenStyle};

/// Name of the language without highlighting.
pub const PLAIN_TEXT: &str = "Plain Text";
/// Lines with more characters are shown without highlighting (and do not
/// change the parser state), as Sublime Text and VS Code do.
pub const MAX_HIGHLIGHT_CHARS: usize = 20_000;

/// Light theme: One Half Light. It and [`DARK_THEME`] are one design in two
/// brightnesses, so a token keeps its hue when the view switches themes;
/// both are tuned for code and ship with two-face/bat.
pub const LIGHT_THEME: EmbeddedThemeName = EmbeddedThemeName::OneHalfLight;
/// Dark theme: One Half Dark (see [`LIGHT_THEME`]).
pub const DARK_THEME: EmbeddedThemeName = EmbeddedThemeName::OneHalfDark;

/// File names (lowercase prefix) that name their language without an
/// extension, checked after the syntaxes' own file names and extensions.
const NAME_PREFIXES: &[(&str, &str)] = &[
    ("dockerfile", "Dockerfile"),
    ("containerfile", "Dockerfile"),
    ("gnumakefile", "Makefile"),
    ("makefile", "Makefile"),
    ("cmakelists", "CMake"),
    ("jenkinsfile", "Groovy"),
    ("vagrantfile", "Ruby"),
    ("gemfile", "Ruby"),
    ("rakefile", "Ruby"),
];

/// A language the highlighter knows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LanguageInfo {
    pub name: String,
    /// File extensions and full file names the language claims.
    pub extensions: Vec<String>,
}

/// Highlighter over one syntax set and the two themes. Cheap to share
/// (`Arc`); each open file gets its own [`HighlightSession`].
pub struct Highlighter {
    syntaxes: SyntaxSet,
    light: Theme,
    dark: Theme,
    problems: Vec<String>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter")
            .field("syntaxes", &self.syntaxes.syntaxes().len())
            .field("problems", &self.problems)
            .finish()
    }
}

impl Highlighter {
    /// Only the built-in languages of two-face.
    pub fn builtin() -> Self {
        Self::with_syntaxes(two_face::syntax::extra_newlines(), Vec::new())
    }

    /// A highlighter over a syntax set built with newline-terminated lines
    /// (`SyntaxDefinition::load_from_str(.., true, ..)`); `problems` are the
    /// German messages about packs that were skipped while loading it.
    pub fn with_syntaxes(syntaxes: SyntaxSet, problems: Vec<String>) -> Self {
        let themes = two_face::theme::extra();
        Self {
            syntaxes,
            light: themes.get(LIGHT_THEME).clone(),
            dark: themes.get(DARK_THEME).clone(),
            problems,
        }
    }

    /// Messages about language packs skipped while loading.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    pub(crate) fn syntax_set(&self) -> &SyntaxSet {
        &self.syntaxes
    }

    pub(crate) fn theme(&self, kind: ThemeKind) -> &Theme {
        match kind {
            ThemeKind::Light => &self.light,
            ThemeKind::Dark => &self.dark,
        }
    }

    /// Every visible language once (an added pack shadows a built-in
    /// language of the same name), sorted by name.
    pub fn languages(&self) -> Vec<LanguageInfo> {
        let mut seen = std::collections::BTreeMap::new();
        for syntax in self.syntaxes.syntaxes().iter().rev() {
            if syntax.hidden {
                continue;
            }
            seen.entry(syntax.name.to_lowercase())
                .or_insert_with(|| LanguageInfo {
                    name: syntax.name.clone(),
                    extensions: syntax.file_extensions.clone(),
                });
        }
        seen.into_values().collect()
    }

    /// The language for a file: its full name (`Makefile`, `.bashrc`), its
    /// extensions from the longest (`d.ts`) to the last (`ts`), known name
    /// prefixes (`Dockerfile.dev`), then the first line (shebang, `<?xml`,
    /// mode line); otherwise [`PLAIN_TEXT`]. Added packs win over built-in
    /// languages because syntect searches the newest definitions first.
    pub fn detect(&self, file_name: &str, first_line: &str) -> String {
        let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        self.detect_by_name(name)
            .or_else(|| self.syntaxes.find_syntax_by_first_line(first_line))
            .map_or_else(|| PLAIN_TEXT.to_string(), |s| s.name.clone())
    }

    fn detect_by_name(&self, name: &str) -> Option<&SyntaxReference> {
        if name.is_empty() {
            return None;
        }
        if let Some(syntax) = self.syntaxes.find_syntax_by_extension(name) {
            return Some(syntax);
        }
        for (dot, _) in name.match_indices('.') {
            let rest = &name[dot + 1..];
            if rest.is_empty() {
                continue;
            }
            if let Some(syntax) = self.syntaxes.find_syntax_by_extension(rest) {
                return Some(syntax);
            }
        }
        let lower = name.to_ascii_lowercase();
        NAME_PREFIXES
            .iter()
            .filter(|(prefix, _)| lower.starts_with(prefix))
            .find_map(|(_, language)| self.syntaxes.find_syntax_by_name(language))
    }

    /// A language named by the user or a Markdown fence: the exact name, the
    /// name in any case, then an extension (`rs`, `cs`).
    pub fn language_by_name(&self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.syntaxes
            .find_syntax_by_name(name)
            .or_else(|| self.syntaxes.find_syntax_by_token(name))
            .map(|syntax| syntax.name.clone())
    }

    /// Index of the syntax a session uses; `None` for plain text.
    pub(crate) fn syntax_index(&self, language: &str) -> Option<usize> {
        let found = self
            .syntaxes
            .syntaxes()
            .iter()
            .rposition(|s| s.name == language)
            .or_else(|| {
                let resolved = self.language_by_name(language)?;
                self.syntaxes
                    .syntaxes()
                    .iter()
                    .rposition(|s| s.name == resolved)
            })?;
        (self.syntaxes.syntaxes()[found].name != PLAIN_TEXT).then_some(found)
    }

    /// A session for one file in `language` (see [`Self::detect`]); an
    /// unknown name gives plain text.
    pub fn session(self: &Arc<Self>, language: &str) -> HighlightSession {
        HighlightSession::new(Arc::clone(self), language)
    }

    /// Highlights a small text at once (Markdown code blocks, previews).
    pub fn highlight_text(
        self: &Arc<Self>,
        language: &str,
        text: &str,
        theme: ThemeKind,
    ) -> Vec<StyledLine> {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let session = self.session(language);
        session
            .highlight(lines.as_slice(), 0, lines.len(), theme)
            .unwrap_or_else(|_| {
                let style = self.default_style(theme);
                lines
                    .into_iter()
                    .map(|line| StyledLine::plain(line, style))
                    .collect()
            })
    }

    /// Style of text no rule colors.
    pub fn default_style(&self, kind: ThemeKind) -> TokenStyle {
        let settings = &self.theme(kind).settings;
        TokenStyle {
            fg: rgba(settings.foreground.unwrap_or(fallback_fg(kind))),
            bg: None,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    /// Colors of the view around the text.
    pub fn theme_colors(&self, kind: ThemeKind) -> ThemeColors {
        let settings = &self.theme(kind).settings;
        let background = settings.background.unwrap_or(fallback_bg(kind));
        let foreground = settings.foreground.unwrap_or(fallback_fg(kind));
        ThemeColors {
            name: match kind {
                ThemeKind::Light => LIGHT_THEME.as_name(),
                ThemeKind::Dark => DARK_THEME.as_name(),
            },
            background: rgba(background),
            foreground: rgba(foreground),
            selection: rgba(settings.selection.unwrap_or(foreground_alpha(foreground))),
            line_highlight: rgba(settings.line_highlight.unwrap_or(background)),
            gutter_foreground: rgba(settings.gutter_foreground.unwrap_or(foreground)),
        }
    }
}

/// Converts a syntect style; the background counts only where it differs
/// from the theme's own background.
pub(crate) fn token_style(style: Style, theme_background: Color) -> TokenStyle {
    TokenStyle {
        fg: rgba(style.foreground),
        bg: (style.background != theme_background).then(|| rgba(style.background)),
        bold: style.font_style.contains(FontStyle::BOLD),
        italic: style.font_style.contains(FontStyle::ITALIC),
        underline: style.font_style.contains(FontStyle::UNDERLINE),
    }
}

fn rgba(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

fn fallback_fg(kind: ThemeKind) -> Color {
    match kind {
        ThemeKind::Light => Color::BLACK,
        ThemeKind::Dark => Color::WHITE,
    }
}

fn fallback_bg(kind: ThemeKind) -> Color {
    match kind {
        ThemeKind::Light => Color::WHITE,
        ThemeKind::Dark => Color::BLACK,
    }
}

fn foreground_alpha(color: Color) -> Color {
    Color { a: 0x40, ..color }
}

/// The process-wide highlighter of the default language folder; rebuilt when
/// the folder changes (also by another process, e.g. `se languages add`).
pub fn shared_highlighter() -> Arc<Highlighter> {
    static SHARED: Mutex<Option<(String, Arc<Highlighter>)>> = Mutex::new(None);
    let store = super::packs::LanguageStore::default_store();
    let fingerprint = store.fingerprint();
    let mut guard = SHARED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((known, highlighter)) = guard.as_ref() {
        if *known == fingerprint {
            return Arc::clone(highlighter);
        }
    }
    let highlighter = Arc::new(store.highlighter());
    *guard = Some((fingerprint, Arc::clone(&highlighter)));
    highlighter
}
