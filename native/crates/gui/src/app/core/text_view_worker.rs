//! Background side of one text view window: opens the document on its place
//! (local file or any backend), builds the line index while the window is
//! already usable, and answers windows of styled lines, the Markdown model
//! and copies of line ranges. Highlighting runs here, never on the UI
//! thread; the window only paints the `StyledLine`s it receives.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use crate::textview::{
    Highlighter, IndexStatus, LanguageInfo, MdDocument, StyledLine, TextDocument, ThemeKind,
    LINE_STRIDE, PLAIN_TEXT,
};

use super::text_tools::TextPlace;

/// What the window asks for.
pub(in crate::app) enum Request {
    /// Lines `first .. first + count`, styled.
    Window {
        first: u64,
        count: usize,
    },
    /// Highlight in this language from now on.
    Language(String),
    Theme(ThemeKind),
    /// The whole file as a Markdown model.
    Markdown,
    /// The text of lines `first .. first + count` for the clipboard.
    Copy {
        first: u64,
        count: usize,
    },
}

/// What the worker answers.
pub(in crate::app) enum Reply {
    Opened {
        doc: Arc<TextDocument>,
        highlighter: Arc<Highlighter>,
        language: String,
        languages: Vec<LanguageInfo>,
    },
    /// The file cannot be shown (German text from `TextViewError`).
    Failed(String),
    Index(IndexStatus),
    IndexFailed(String),
    /// `generation` counts the language and theme changes the worker applied.
    Lines {
        first: u64,
        generation: u64,
        lines: Vec<StyledLine>,
    },
    Markdown {
        generation: u64,
        doc: MdDocument,
    },
    Copied(String),
    Error(String),
}

/// Starts the worker of one window; it ends when the request sender is
/// dropped (window closed) or `cancel` is set.
pub(in crate::app) fn spawn(
    place: TextPlace,
    path: String,
    theme: ThemeKind,
    cancel: Arc<AtomicBool>,
) -> Result<(Sender<Request>, Receiver<Reply>), String> {
    let (request_tx, requests) = crossbeam_channel::unbounded();
    let (reply_tx, replies) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("text-view".into())
        .spawn(move || run(place, path, theme, cancel, requests, reply_tx))
        .map_err(|error| format!("Die Ansicht konnte nicht starten: {error}"))?;
    Ok((request_tx, replies))
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn open_document(place: &TextPlace, path: &str) -> Result<TextDocument, String> {
    let opened = match &place.backend {
        None => crate::textview::open_local(std::path::Path::new(path)),
        Some(backend) => crate::textview::open_backend(backend.clone(), path),
    };
    opened.map_err(|error| error.to_string())
}

/// The worker of a window over a document already in memory (tests and the
/// GUI task harness).
#[cfg(test)]
pub(in crate::app) fn spawn_document(
    doc: TextDocument,
    path: String,
    theme: ThemeKind,
    cancel: Arc<AtomicBool>,
) -> Result<(Sender<Request>, Receiver<Reply>), String> {
    let (request_tx, requests) = crossbeam_channel::unbounded();
    let (reply_tx, replies) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("text-view".into())
        .spawn(move || serve(Arc::new(doc), path, theme, cancel, requests, reply_tx))
        .map_err(|error| format!("Die Ansicht konnte nicht starten: {error}"))?;
    Ok((request_tx, replies))
}

fn run(
    place: TextPlace,
    path: String,
    theme: ThemeKind,
    cancel: Arc<AtomicBool>,
    requests: Receiver<Request>,
    replies: Sender<Reply>,
) {
    match open_document(&place, &path) {
        Ok(doc) => serve(Arc::new(doc), path, theme, cancel, requests, replies),
        Err(error) => {
            let _ = replies.send(Reply::Failed(error));
        }
    }
}

fn serve(
    doc: Arc<TextDocument>,
    path: String,
    mut theme: ThemeKind,
    cancel: Arc<AtomicBool>,
    requests: Receiver<Request>,
    replies: Sender<Reply>,
) {
    let highlighter = crate::textview::shared_highlighter();
    let first_line = doc
        .lines(0, 1)
        .ok()
        .and_then(|mut lines| lines.pop())
        .unwrap_or_default();
    let mut language = highlighter.detect(file_name(&path), &first_line);
    let _ = replies.send(Reply::Opened {
        doc: doc.clone(),
        highlighter: highlighter.clone(),
        language: language.clone(),
        languages: highlighter.languages(),
    });
    spawn_index(doc.clone(), cancel.clone(), replies.clone());
    let mut session = highlighter.session(&language);
    let mut generation = 0u64;
    while let Ok(first) = requests.recv() {
        let mut window = None;
        for request in std::iter::once(first).chain(requests.try_iter()) {
            match request {
                Request::Window { first, count } => window = Some((first, count)),
                Request::Language(name) => {
                    language = name;
                    session = highlighter.session(&language);
                    generation += 1;
                }
                Request::Theme(kind) => {
                    theme = kind;
                    generation += 1;
                }
                Request::Markdown => {
                    let reply = match doc.lines(0, usize::MAX) {
                        Ok(lines) => Reply::Markdown {
                            generation,
                            doc: crate::textview::parse_markdown(
                                &lines.join("\n"),
                                &highlighter,
                                theme,
                            ),
                        },
                        Err(error) => Reply::Error(format!("Markdown lesen: {error}")),
                    };
                    let _ = replies.send(reply);
                }
                Request::Copy { first, count } => {
                    let reply = match doc.lines(first, count) {
                        Ok(lines) => Reply::Copied(lines.join("\n")),
                        Err(error) => Reply::Error(format!("Zeilen kopieren: {error}")),
                    };
                    let _ = replies.send(reply);
                }
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some((first, count)) = window else {
            continue;
        };
        // Far below the highlighter's checkpoints the parse from the nearest
        // one takes a while: the plain lines come first.
        let covered = session.checkpoint_count() as u64 * LINE_STRIDE;
        if session.language() != PLAIN_TEXT && first > covered {
            let style = highlighter.default_style(theme);
            if let Ok(lines) = doc.lines(first, count) {
                let lines = lines
                    .into_iter()
                    .map(|text| StyledLine::plain(text, style))
                    .collect();
                let _ = replies.send(Reply::Lines {
                    first,
                    generation,
                    lines,
                });
            }
        }
        let reply = match session.highlight(doc.as_ref(), first, count, theme) {
            Ok(lines) => Reply::Lines {
                first,
                generation,
                lines,
            },
            Err(error) => Reply::Error(format!("Zeilen lesen: {error}")),
        };
        if replies.send(reply).is_err() {
            return;
        }
    }
}

fn spawn_index(doc: Arc<TextDocument>, cancel: Arc<AtomicBool>, replies: Sender<Reply>) {
    let failed = replies.clone();
    let started = std::thread::Builder::new()
        .name("text-view-index".into())
        .spawn(move || {
            let mut report = |status: &IndexStatus| {
                let _ = replies.send(Reply::Index(status.clone()));
            };
            match doc.build_index(cancel.as_ref(), &mut report) {
                Ok(status) => {
                    let _ = replies.send(Reply::Index(status));
                }
                Err(crate::textview::TextViewError::Cancelled) => {}
                Err(error) => {
                    let _ = replies.send(Reply::IndexFailed(error.to_string()));
                }
            }
        });
    if let Err(error) = started {
        let _ = failed.send(Reply::IndexFailed(format!(
            "Zeilenindex konnte nicht starten: {error}"
        )));
    }
}
