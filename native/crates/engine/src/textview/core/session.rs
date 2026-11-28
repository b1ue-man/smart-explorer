//! Highlighting of one file of any size. The parser state is kept every
//! [`LINE_STRIDE`] lines (a checkpoint); visible lines are computed from the
//! nearest checkpoint before them, and [`HighlightSession::prepare`] fills
//! the checkpoints on a background thread. The result is the same as one
//! pass from the top: checkpoints hold the exact parser state and scope
//! stack, and the theme's styles are derived from the scope stack alone.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use syntect::highlighting::{
    HighlightState, Highlighter as ThemeHighlighter, RangedHighlightIterator,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use super::document::{LineProvider, LINE_STRIDE};
use super::error::{TextViewError, TextViewResult};
use super::highlight::{token_style, Highlighter, MAX_HIGHLIGHT_CHARS, PLAIN_TEXT};
use super::style::{StyledLine, StyledSpan, ThemeKind, TokenStyle};

#[derive(Clone)]
struct Checkpoint {
    parse: ParseState,
    scopes: ScopeStack,
}

/// Highlighting state of one open file in one language.
pub struct HighlightSession {
    highlighter: Arc<Highlighter>,
    syntax: Option<usize>,
    language: String,
    /// Checkpoint `k` is the state before line `k * LINE_STRIDE`.
    checkpoints: Mutex<Vec<Checkpoint>>,
}

impl std::fmt::Debug for HighlightSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightSession")
            .field("language", &self.language)
            .field("checkpoints", &self.checkpoint_count())
            .finish()
    }
}

impl HighlightSession {
    pub(crate) fn new(highlighter: Arc<Highlighter>, language: &str) -> Self {
        let syntax = highlighter.syntax_index(language);
        let language = syntax.map_or_else(
            || PLAIN_TEXT.to_string(),
            |i| highlighter.syntax_set().syntaxes()[i].name.clone(),
        );
        let checkpoints = syntax
            .map(|i| {
                vec![Checkpoint {
                    parse: ParseState::new(&highlighter.syntax_set().syntaxes()[i]),
                    scopes: ScopeStack::new(),
                }]
            })
            .unwrap_or_default();
        Self {
            highlighter,
            syntax,
            language,
            checkpoints: Mutex::new(checkpoints),
        }
    }

    /// The language actually used ([`PLAIN_TEXT`] for unknown names).
    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn highlighter(&self) -> &Arc<Highlighter> {
        &self.highlighter
    }

    /// Number of checkpoints known (lines covered = count × LINE_STRIDE).
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Lines `first .. first + count` of `lines` styled with `theme`. Lines
    /// between the nearest checkpoint and `first` are parsed (not styled);
    /// checkpoints met on the way are stored.
    pub fn highlight(
        &self,
        lines: &(impl LineProvider + ?Sized),
        first: u64,
        count: usize,
        theme: ThemeKind,
    ) -> io::Result<Vec<StyledLine>> {
        let default = self.highlighter.default_style(theme);
        let Some(index) = self.syntax else {
            return Ok(lines
                .provide_lines(first, count)?
                .into_iter()
                .map(|text| StyledLine::plain(text, default))
                .collect());
        };
        let set = self.highlighter.syntax_set();
        let syntax = &set.syntaxes()[index];
        let (mut line_no, mut state) = self.nearest(first);
        while line_no < first {
            let to_boundary = LINE_STRIDE - line_no % LINE_STRIDE;
            let want = to_boundary.min(first - line_no) as usize;
            let batch = lines.provide_lines(line_no, want)?;
            if batch.is_empty() {
                return Ok(Vec::new());
            }
            for text in &batch {
                advance(&mut state, text, syntax, set);
                line_no += 1;
                self.store(line_no, &state);
            }
        }
        let texts = lines.provide_lines(first, count)?;
        let theme_highlighter = ThemeHighlighter::new(self.highlighter.theme(theme));
        let theme_background = theme_highlighter.get_default().background;
        let mut highlight = HighlightState::new(&theme_highlighter, state.scopes);
        let mut parse = state.parse;
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let styled = style_line(
                &mut parse,
                &mut highlight,
                text,
                LineContext {
                    syntax,
                    set,
                    theme: &theme_highlighter,
                    theme_background,
                    default,
                },
            );
            out.push(styled);
            line_no += 1;
            if line_no % LINE_STRIDE == 0 {
                self.store(
                    line_no,
                    &Checkpoint {
                        parse: parse.clone(),
                        scopes: highlight.path.clone(),
                    },
                );
            }
        }
        Ok(out)
    }

    /// Fills the checkpoints up to line `until` (`None`: the whole file),
    /// 1024 lines per step; returns the number of lines covered. Run it on a
    /// background thread; setting `cancel` stops it with
    /// [`TextViewError::Cancelled`], and a later call continues.
    pub fn prepare(
        &self,
        lines: &(impl LineProvider + ?Sized),
        until: Option<u64>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> TextViewResult<u64> {
        let Some(index) = self.syntax else {
            return Ok(until.unwrap_or(0));
        };
        let set = self.highlighter.syntax_set();
        let syntax = &set.syntaxes()[index];
        let target = until.unwrap_or(u64::MAX);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(TextViewError::Cancelled);
            }
            let (known, mut state) = {
                let checkpoints = self
                    .checkpoints
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let k = checkpoints.len() - 1;
                (k as u64 * LINE_STRIDE, checkpoints[k].clone())
            };
            if known >= target {
                return Ok(known);
            }
            let batch = lines.provide_lines(known, LINE_STRIDE as usize)?;
            for text in &batch {
                advance(&mut state, text, syntax, set);
            }
            let reached = known + batch.len() as u64;
            if (batch.len() as u64) < LINE_STRIDE {
                return Ok(reached);
            }
            self.store(reached, &state);
            progress(reached);
        }
    }

    fn nearest(&self, line: u64) -> (u64, Checkpoint) {
        let checkpoints = self
            .checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let k = usize::try_from(line / LINE_STRIDE)
            .unwrap_or(usize::MAX)
            .min(checkpoints.len() - 1);
        (k as u64 * LINE_STRIDE, checkpoints[k].clone())
    }

    /// Stores the state before `line` when it is the next missing checkpoint.
    fn store(&self, line: u64, state: &Checkpoint) {
        if line % LINE_STRIDE != 0 {
            return;
        }
        let mut checkpoints = self
            .checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if checkpoints.len() as u64 == line / LINE_STRIDE {
            checkpoints.push(state.clone());
        }
    }
}

fn too_long(text: &str) -> bool {
    text.len() > MAX_HIGHLIGHT_CHARS && text.chars().nth(MAX_HIGHLIGHT_CHARS).is_some()
}

/// Parses one line without styling it (the scope stack is kept up to date).
fn advance(state: &mut Checkpoint, text: &str, syntax: &SyntaxReference, set: &SyntaxSet) {
    if too_long(text) {
        return;
    }
    let line = format!("{text}\n");
    match state.parse.parse_line(&line, set) {
        Ok(ops) => {
            for (_, op) in &ops {
                // The highlight iterator stops at the first scope error too.
                if state.scopes.apply(op).is_err() {
                    break;
                }
            }
        }
        Err(_) => {
            state.parse = ParseState::new(syntax);
            state.scopes = ScopeStack::new();
        }
    }
}

struct LineContext<'a> {
    syntax: &'a SyntaxReference,
    set: &'a SyntaxSet,
    theme: &'a ThemeHighlighter<'a>,
    theme_background: syntect::highlighting::Color,
    default: TokenStyle,
}

/// Parses and styles one line. A parse error resets the state (as
/// [`advance`] does) and shows the line plain.
fn style_line(
    parse: &mut ParseState,
    highlight: &mut HighlightState,
    text: String,
    cx: LineContext<'_>,
) -> StyledLine {
    if too_long(&text) {
        return StyledLine::plain(text, cx.default);
    }
    let line = format!("{text}\n");
    let ops = match parse.parse_line(&line, cx.set) {
        Ok(ops) => ops,
        Err(_) => {
            *parse = ParseState::new(cx.syntax);
            *highlight = HighlightState::new(cx.theme, ScopeStack::new());
            return StyledLine::plain(text, cx.default);
        }
    };
    let mut spans: Vec<StyledSpan> = Vec::new();
    for (style, _, range) in RangedHighlightIterator::new(highlight, &ops, &line, cx.theme) {
        let end = range.end.min(text.len());
        if range.start >= end {
            continue;
        }
        push_span(
            &mut spans,
            range.start,
            end,
            token_style(style, cx.theme_background),
        );
    }
    let covered = spans.last().map_or(0, |span| span.end);
    if covered < text.len() {
        push_span(&mut spans, covered, text.len(), cx.default);
    }
    StyledLine { text, spans }
}

fn push_span(spans: &mut Vec<StyledSpan>, start: usize, end: usize, style: TokenStyle) {
    if let Some(last) = spans.last_mut() {
        if last.style == style && last.end == start {
            last.end = end;
            return;
        }
    }
    spans.push(StyledSpan { start, end, style });
}
