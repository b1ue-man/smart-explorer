//! State of one text view window („Ansicht“): the document from its worker,
//! the window of styled lines around the visible part, the line position,
//! find and go-to state and the Markdown view. Free of drawing; the window
//! itself is `text_view_ui.rs`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use crate::textview::{
    Highlighter, IndexStatus, LanguageInfo, MdDocument, StyledLine, TextDocument, ThemeKind,
};

use super::md_images::MdImages;
use super::text_tools::TextPlace;
use super::text_view_find::{FindHit, FindOutcome};
use super::text_view_worker::{Reply, Request};

/// Lines loaded above and below the visible ones.
const MARGIN_LINES: u64 = 200;

pub(in crate::app) enum ViewState {
    Loading,
    Ready,
    Failed(String),
}

/// Find bar (Strg+F).
#[derive(Default)]
pub(in crate::app) struct FindBar {
    pub(in crate::app) open: bool,
    pub(in crate::app) text: String,
    pub(in crate::app) regex: bool,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) current: Option<FindHit>,
    pub(in crate::app) status: Option<String>,
    pub(in crate::app) focus: bool,
    pub(in crate::app) job: Option<(Receiver<std::io::Result<FindOutcome>>, Arc<AtomicBool>)>,
}

/// Go-to bar (Strg+G).
#[derive(Default)]
pub(in crate::app) struct GotoBar {
    pub(in crate::app) open: bool,
    pub(in crate::app) text: String,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) focus: bool,
}

/// The rendered Markdown of a `.md` file.
#[derive(Default)]
pub(in crate::app) struct MdView {
    pub(in crate::app) rendered: bool,
    pub(in crate::app) doc: Option<MdDocument>,
    pub(in crate::app) loaded_generation: Option<u64>,
    pub(in crate::app) requested: bool,
    pub(in crate::app) images: MdImages,
    /// Heading slug a `#…` link asked for; scrolled to in the next frame.
    pub(in crate::app) anchor: Option<String>,
}

pub(in crate::app) fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

pub(in crate::app) struct TextViewWindow {
    pub(in crate::app) id: u64,
    pub(in crate::app) place: TextPlace,
    pub(in crate::app) path: String,
    pub(in crate::app) name: String,
    pub(in crate::app) open: bool,
    pub(in crate::app) state: ViewState,
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
    pub(in crate::app) doc: Option<Arc<TextDocument>>,
    pub(in crate::app) highlighter: Option<Arc<Highlighter>>,
    pub(in crate::app) languages: Vec<LanguageInfo>,
    pub(in crate::app) language: String,
    pub(in crate::app) status: Option<IndexStatus>,
    pub(in crate::app) index_error: Option<String>,
    theme: Option<ThemeKind>,
    generation: u64,
    cache_first: u64,
    cache: Vec<StyledLine>,
    cache_generation: u64,
    /// The worker sent fewer lines than asked: the cache ends with the file.
    cache_short: bool,
    pending: Option<(u64, usize, u64)>,
    /// First line shown (zero-based) and the wheel remainder in lines.
    pub(in crate::app) top: u64,
    pub(in crate::app) scroll_rest: f32,
    /// Rows that fit the body (set while drawing).
    pub(in crate::app) visible_rows: usize,
    pub(in crate::app) wrap: bool,
    /// The marked line (go-to target, search hit) and its marked byte ranges.
    pub(in crate::app) cursor: Option<u64>,
    pub(in crate::app) marks: Vec<(usize, usize)>,
    /// Selected lines (anchor, end), both inclusive.
    pub(in crate::app) selection: Option<(u64, u64)>,
    pub(in crate::app) find: FindBar,
    pub(in crate::app) goto: GotoBar,
    pub(in crate::app) markdown: Option<MdView>,
    pub(in crate::app) message: Option<String>,
}

impl TextViewWindow {
    /// Opens `path` on `place`; `line` (zero-based) and its marks come from a
    /// search hit. The window is usable at once; the worker opens the file.
    pub(in crate::app) fn open(
        id: u64,
        place: TextPlace,
        path: String,
        theme: ThemeKind,
        line: Option<(u64, Vec<(usize, usize)>)>,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let spawned =
            super::text_view_worker::spawn(place.clone(), path.clone(), theme, cancel.clone());
        Self::assemble(id, place, path, theme, line, spawned, cancel)
    }

    fn assemble(
        id: u64,
        place: TextPlace,
        path: String,
        theme: ThemeKind,
        line: Option<(u64, Vec<(usize, usize)>)>,
        spawned: Result<(Sender<Request>, Receiver<Reply>), String>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let (requests, replies, state) = match spawned {
            Ok((requests, replies)) => (requests, replies, ViewState::Loading),
            Err(error) => {
                let (requests, _) = crossbeam_channel::unbounded();
                let (_, replies) = crossbeam_channel::unbounded();
                (requests, replies, ViewState::Failed(error))
            }
        };
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        let markdown = is_markdown(&path).then(|| MdView {
            rendered: line.is_none(),
            ..MdView::default()
        });
        let (cursor, marks) = match line {
            Some((line, marks)) => (Some(line), marks),
            None => (None, Vec::new()),
        };
        let mut window = Self {
            id,
            place,
            path,
            name,
            open: true,
            state,
            requests,
            replies,
            cancel,
            doc: None,
            highlighter: None,
            languages: Vec::new(),
            language: crate::textview::PLAIN_TEXT.to_string(),
            status: None,
            index_error: None,
            theme: Some(theme),
            generation: 0,
            cache_first: 0,
            cache: Vec::new(),
            cache_generation: 0,
            cache_short: false,
            pending: None,
            top: 0,
            scroll_rest: 0.0,
            visible_rows: 30,
            wrap: false,
            cursor,
            marks,
            selection: None,
            find: FindBar::default(),
            goto: GotoBar::default(),
            markdown,
            message: None,
        };
        if let Some(line) = cursor {
            window.jump_to(line);
        }
        window
    }

    /// A window over a document in memory (tests and the GUI task harness);
    /// its worker serves the document like an opened file.
    #[cfg(test)]
    pub(in crate::app) fn from_document(id: u64, path: &str, doc: TextDocument) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let spawned = super::text_view_worker::spawn_document(
            doc,
            path.to_string(),
            ThemeKind::Light,
            cancel.clone(),
        );
        Self::assemble(
            id,
            TextPlace::local(),
            path.to_string(),
            ThemeKind::Light,
            None,
            spawned,
            cancel,
        )
    }

    pub(in crate::app) fn window_id(&self) -> eframe::egui::Id {
        eframe::egui::Id::new(("text_view", self.id))
    }

    pub(in crate::app) fn same_file(&self, place: &TextPlace, path: &str) -> bool {
        self.path == path && self.place.same(place)
    }

    /// Lines known so far: the final count once the index is complete.
    pub(in crate::app) fn known_lines(&self) -> u64 {
        let indexed = match &self.status {
            Some(status) if status.complete => return status.lines.max(1),
            Some(status) => status.lines + 1,
            None => 1,
        };
        indexed.max(self.cache_first + self.cache.len() as u64)
    }

    pub(in crate::app) fn index_complete(&self) -> bool {
        self.status.as_ref().is_some_and(|status| status.complete)
    }

    /// Takes the worker's answers.
    pub(in crate::app) fn poll(&mut self, ctx: &eframe::egui::Context) {
        for reply in self.replies.try_iter().collect::<Vec<_>>() {
            match reply {
                Reply::Opened {
                    doc,
                    highlighter,
                    language,
                    languages,
                } => {
                    self.doc = Some(doc);
                    self.highlighter = Some(highlighter);
                    self.language = language;
                    self.languages = languages;
                    self.state = ViewState::Ready;
                }
                Reply::Failed(error) => self.state = ViewState::Failed(error),
                Reply::Index(status) => self.status = Some(status),
                Reply::IndexFailed(error) => self.index_error = Some(error),
                Reply::Lines {
                    first,
                    generation,
                    lines,
                    ..
                } => {
                    if generation == self.generation {
                        let asked = self
                            .pending
                            .filter(|(at, _, _)| *at == first)
                            .map(|(_, count, _)| count);
                        match asked {
                            Some(count) => self.cache_short = lines.len() < count,
                            None if first != self.cache_first => self.cache_short = false,
                            None => {}
                        }
                        self.cache_first = first;
                        self.cache = lines;
                        self.cache_generation = generation;
                        if asked.is_some() {
                            self.pending = None;
                        }
                    }
                }
                Reply::Markdown { generation, doc } => {
                    if let Some(markdown) = &mut self.markdown {
                        markdown.doc = Some(doc);
                        markdown.loaded_generation = Some(generation);
                        markdown.requested = false;
                    }
                }
                Reply::Copied(text) => {
                    ctx.copy_text(text);
                    self.message = Some("Zeilen kopiert.".to_string());
                }
                Reply::Error(error) => self.message = Some(error),
            }
        }
        self.poll_find();
        if let Some(markdown) = &mut self.markdown {
            markdown.images.poll(ctx);
        }
    }

    pub(in crate::app) fn busy(&self) -> bool {
        matches!(self.state, ViewState::Loading)
            || self.pending.is_some()
            || self.find.job.is_some()
            || (self.doc.is_some() && !self.index_complete() && self.index_error.is_none())
            || self
                .markdown
                .as_ref()
                .is_some_and(|markdown| markdown.requested)
    }

    /// Follows the colour scheme of the app.
    pub(in crate::app) fn set_theme(&mut self, theme: ThemeKind) {
        if self.theme == Some(theme) {
            return;
        }
        self.theme = Some(theme);
        self.send(Request::Theme(theme));
        if let Some(markdown) = &mut self.markdown {
            markdown.loaded_generation = None;
        }
    }

    pub(in crate::app) fn theme(&self) -> ThemeKind {
        self.theme.unwrap_or_default()
    }

    pub(in crate::app) fn set_language(&mut self, language: String) {
        if language == self.language {
            return;
        }
        self.language = language.clone();
        self.send(Request::Language(language));
    }

    fn send(&mut self, request: Request) {
        if matches!(request, Request::Language(_) | Request::Theme(_)) {
            self.generation += 1;
            self.pending = None;
        }
        let _ = self.requests.send(request);
    }

    /// Makes sure the lines `first .. first + count` are (being) loaded.
    pub(in crate::app) fn ensure_lines(&mut self, first: u64, count: usize) {
        if self.doc.is_none() {
            return;
        }
        let end = first + count as u64;
        let cached_end = self.cache_first + self.cache.len() as u64;
        let fresh = self.cache_generation == self.generation;
        let covered =
            fresh && first >= self.cache_first && (end <= cached_end || self.at_file_end());
        if covered {
            return;
        }
        let want_first = first.saturating_sub(MARGIN_LINES);
        let want_count = count + 2 * MARGIN_LINES as usize;
        if self.pending.is_some_and(|(at, n, generation)| {
            generation == self.generation && at <= first && at + n as u64 >= end
        }) {
            return;
        }
        self.pending = Some((want_first, want_count, self.generation));
        let _ = self.requests.send(Request::Window {
            first: want_first,
            count: want_count,
        });
    }

    /// The cache reaches the end of the file (fewer lines came than asked).
    fn at_file_end(&self) -> bool {
        self.cache_short
            || (self.index_complete()
                && self.cache_first + self.cache.len() as u64 >= self.known_lines())
    }

    /// A loaded line; after a language or theme change the old colours stay
    /// until the new ones arrive.
    pub(in crate::app) fn line(&self, number: u64) -> Option<&StyledLine> {
        let offset = number.checked_sub(self.cache_first)?;
        self.cache.get(usize::try_from(offset).ok()?)
    }

    /// Highest first line for the current body height.
    pub(in crate::app) fn max_top(&self) -> u64 {
        let known = self.known_lines();
        if self.wrap {
            known.saturating_sub(1)
        } else {
            known.saturating_sub(self.visible_rows.max(1) as u64)
        }
    }

    pub(in crate::app) fn scroll_to(&mut self, top: u64) {
        self.top = top.min(self.max_top());
    }

    pub(in crate::app) fn scroll_by(&mut self, lines: i64) {
        let top = if lines < 0 {
            self.top.saturating_sub(lines.unsigned_abs())
        } else {
            self.top.saturating_add(lines as u64)
        };
        self.scroll_to(top);
    }

    /// Marks `line` and scrolls so it stands in the upper third.
    pub(in crate::app) fn jump_to(&mut self, line: u64) {
        self.cursor = Some(line);
        let top = line.saturating_sub(self.visible_rows as u64 / 3);
        // Before the index reaches the line, max_top would hold it back.
        self.top = if line >= self.known_lines() {
            top
        } else {
            top.min(self.max_top())
        };
    }

    /// Strg+C over the selected lines (read by the worker, any size).
    pub(in crate::app) fn copy_selection(&mut self) {
        if let Some((a, b)) = self.selection {
            let (first, last) = (a.min(b), a.max(b));
            let count = usize::try_from(last - first + 1).unwrap_or(usize::MAX);
            let _ = self.requests.send(Request::Copy { first, count });
        }
    }

    /// Asks for the Markdown model when the rendered view needs it.
    pub(in crate::app) fn ensure_markdown(&mut self) {
        let generation = self.generation;
        let Some(markdown) = &mut self.markdown else {
            return;
        };
        if self.doc.is_none()
            || markdown.requested
            || markdown.loaded_generation == Some(generation)
        {
            return;
        }
        markdown.requested = true;
        let _ = self.requests.send(Request::Markdown);
    }
}

impl Drop for TextViewWindow {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some((_, cancel)) = &self.find.job {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
