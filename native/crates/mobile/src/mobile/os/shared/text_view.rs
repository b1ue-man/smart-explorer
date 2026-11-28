//! `text.*` (api.md §4.15): the text view of the app over the core's
//! `textview` (plan docs/plaene/2026-11-27-dateien-suche-ansicht, part D).
//! An open document reads its file in windows (local files with positioned
//! reads, remote ones through the pooled backend), builds its line index and
//! the highlighting checkpoints on a background thread and hands out styled
//! lines; the app knows no languages, only spans and colors. At most
//! `MAX_DOCS` documents stay open; the least recently used one goes first.
use super::args::{bool_or, opt_str, str_arg, u64_or};
use super::entry::extension;
use super::error::ApiError;
use super::fs_list::reject_trash;
use super::location::Loc;
use super::runtime::{lock, Runtime};
use super::text_find::{find, FindRequest, Found};
use super::text_json::{colors_json, markdown_json, styled_line_json};
use crate::agent_proto::{ContentQuery, TextMode};
use crate::textview::{
    open_backend, open_local, parse_markdown, shared_highlighter, HighlightSession, TextDocument,
    TextViewError, ThemeKind, PLAIN_TEXT,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Documents kept open at once.
pub(crate) const MAX_DOCS: usize = 8;
/// Lines one `text.lines` call returns at most.
const MAX_LINES_PER_CALL: usize = 1000;
/// Larger documents are not rendered as Markdown (the source view stays).
const MAX_MARKDOWN_BYTES: u64 = 16 * 1024 * 1024;
/// Lines read per step when a whole document is read.
const READ_STEP: usize = 4096;

#[derive(Default)]
pub(crate) struct TextDocs {
    docs: Mutex<Vec<(String, Arc<OpenDoc>)>>,
    next: AtomicU64,
}

impl TextDocs {
    /// Registers `doc` and returns its id; beyond `MAX_DOCS` the least
    /// recently used document is closed.
    pub(crate) fn insert(&self, doc: Arc<OpenDoc>) -> String {
        let id = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let mut docs = lock(&self.docs);
        docs.push((id.clone(), doc));
        while docs.len() > MAX_DOCS {
            let oldest = docs
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, doc))| *lock(&doc.last_used))
                .map(|(index, _)| index)
                .unwrap_or(0);
            let (_, gone) = docs.remove(oldest);
            gone.cancel.store(true, Ordering::Release);
        }
        id
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<OpenDoc>, ApiError> {
        let docs = lock(&self.docs);
        let doc = docs
            .iter()
            .find(|(doc_id, _)| doc_id == id)
            .map(|(_, doc)| doc.clone())
            .ok_or_else(|| ApiError::not_found("Dieses Dokument ist nicht mehr geöffnet"))?;
        *lock(&doc.last_used) = Instant::now();
        Ok(doc)
    }

    /// Closes `id`; false when it was not open.
    pub(crate) fn remove(&self, id: &str) -> bool {
        let mut docs = lock(&self.docs);
        let Some(index) = docs.iter().position(|(doc_id, _)| doc_id == id) else {
            return false;
        };
        let (_, doc) = docs.remove(index);
        doc.cancel.store(true, Ordering::Release);
        true
    }

    #[cfg(test)]
    pub(crate) fn ids(&self) -> Vec<String> {
        lock(&self.docs).iter().map(|(id, _)| id.clone()).collect()
    }
}

/// The highlighting of one document in one language, with the flag that
/// stops its background preparation.
struct Highlighting {
    session: Arc<HighlightSession>,
    stop: Arc<AtomicBool>,
}

pub(crate) struct OpenDoc {
    doc: Arc<TextDocument>,
    name: String,
    /// Language found for the file (name, first line).
    detected: String,
    /// Set on close: ends the background work of this document.
    cancel: Arc<AtomicBool>,
    index_error: Mutex<Option<String>>,
    highlighting: Mutex<Option<Highlighting>>,
    last_used: Mutex<Instant>,
}

impl OpenDoc {
    pub(crate) fn new(doc: TextDocument, name: &str) -> Arc<Self> {
        let first = doc
            .lines(0, 1)
            .ok()
            .and_then(|lines| lines.into_iter().next())
            .unwrap_or_default();
        let detected = shared_highlighter().detect(name, &first);
        Arc::new(Self {
            doc: Arc::new(doc),
            name: name.to_string(),
            detected,
            cancel: Arc::new(AtomicBool::new(false)),
            index_error: Mutex::new(None),
            highlighting: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        })
    }

    fn is_markdown(&self) -> bool {
        self.detected.eq_ignore_ascii_case("Markdown")
            || matches!(
                extension(&self.name, false).as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mkdn"
            )
    }

    /// Builds the line index, then prepares the highlighting, on a thread of
    /// its own; a failure is kept for `text.status`.
    fn start_background(self: &Arc<Self>) {
        let open = Arc::clone(self);
        let spawned =
            std::thread::Builder::new()
                .name("text-index".into())
                .spawn(
                    move || match open.doc.build_index(&open.cancel, &mut |_| {}) {
                        Ok(_) => open.prepare_current(),
                        Err(TextViewError::Cancelled) => {}
                        Err(error) => *lock(&open.index_error) = Some(error.to_string()),
                    },
                );
        if let Err(error) = spawned {
            *lock(&self.index_error) = Some(format!("Zeilenindex nicht gestartet: {error}"));
        }
    }

    /// Fills the checkpoints of the current highlighting up to the end, so a
    /// jump far down the file does not parse everything in one call.
    fn prepare_current(&self) {
        let Some((session, stop)) = lock(&self.highlighting)
            .as_ref()
            .map(|h| (Arc::clone(&h.session), Arc::clone(&h.stop)))
        else {
            return;
        };
        let stop_both = StopEither(&self.cancel, &stop);
        let flag = AtomicBool::new(stop_both.stopped());
        let _ = session.prepare(&*self.doc, None, &flag, &mut |_| {
            if stop_both.stopped() {
                flag.store(true, Ordering::Relaxed);
            }
        });
    }

    /// The highlighting for `language` (`None` or empty: the detected one);
    /// a new one when the language or the installed language packs changed.
    fn session(self: &Arc<Self>, language: Option<&str>) -> Arc<HighlightSession> {
        let highlighter = shared_highlighter();
        let wanted = language
            .map(str::trim)
            .filter(|language| !language.is_empty())
            .unwrap_or(&self.detected)
            .to_string();
        let resolved = highlighter
            .language_by_name(&wanted)
            .unwrap_or_else(|| PLAIN_TEXT.to_string());
        let mut slot = lock(&self.highlighting);
        if let Some(current) = slot.as_ref() {
            if Arc::ptr_eq(current.session.highlighter(), &highlighter)
                && current.session.language() == resolved
            {
                return Arc::clone(&current.session);
            }
            current.stop.store(true, Ordering::Release);
        }
        let session = Arc::new(highlighter.session(&resolved));
        *slot = Some(Highlighting {
            session: Arc::clone(&session),
            stop: Arc::new(AtomicBool::new(false)),
        });
        drop(slot);
        if self.doc.line_count().is_some() {
            let open = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("text-prepare".into())
                .spawn(move || open.prepare_current());
        }
        session
    }

    fn status_json(&self) -> Value {
        let status = self.doc.status();
        json!({
            "lines": status.lines,
            "complete": status.complete,
            "bytesDone": status.bytes_done,
            "totalBytes": status.total_bytes,
            "encoding": status.encoding.label(),
            "encodingKey": status.encoding,
            "endings": status.endings,
            "lineEnding": status.endings.dominant().map(|ending| ending.label()),
            "mixedEndings": status.endings.is_mixed(),
            "error": *lock(&self.index_error),
        })
    }

    /// The whole text, lines joined with `\n`.
    fn whole_text(&self) -> Result<String, ApiError> {
        if self.doc.len() > MAX_MARKDOWN_BYTES {
            return Err(ApiError::unsupported(
                "Die Datei ist zu groß für die Markdown-Darstellung (über 16 MiB); der Quelltext bleibt lesbar",
            ));
        }
        let mut lines = Vec::new();
        loop {
            let step = self.doc.lines(lines.len() as u64, READ_STEP)?;
            let read = step.len();
            lines.extend(step);
            if read < READ_STEP {
                return Ok(lines.join("\n"));
            }
        }
    }
}

/// Stops when either flag is set.
struct StopEither<'a>(&'a AtomicBool, &'a AtomicBool);

impl StopEither<'_> {
    fn stopped(&self) -> bool {
        self.0.load(Ordering::Acquire) || self.1.load(Ordering::Acquire)
    }
}

/// German error of the text view as a bridge error.
pub(crate) fn text_error(error: TextViewError) -> ApiError {
    let message = error.to_string();
    match error {
        TextViewError::Io(error) => ApiError::from(error),
        TextViewError::Cancelled => ApiError::canceled(),
        TextViewError::NotText => ApiError::unsupported(message),
        TextViewError::PackExists(_) => ApiError::new("exists", message),
        TextViewError::PackNotFound(_) => ApiError::not_found(message),
        TextViewError::InvalidPack(_)
        | TextViewError::InvalidPackName(_)
        | TextViewError::BuiltinPack(_) => ApiError::invalid(message),
    }
}

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "text.open" => open(rt, args),
        "text.status" => doc(rt, args).map(|open| open.status_json()),
        "text.lines" => lines(rt, args),
        "text.find" => find_in(rt, args),
        "text.markdown" => markdown(rt, args),
        "text.close" => {
            str_arg(args, "docId").map(|id| json!({ "closed": rt.inner.texts.remove(id) }))
        }
        "text.languages" => Ok(languages()),
        _ => return None,
    })
}

fn doc(rt: &Runtime, args: &Value) -> Result<Arc<OpenDoc>, ApiError> {
    rt.inner.texts.get(str_arg(args, "docId")?)
}

fn theme(args: &Value) -> Result<ThemeKind, ApiError> {
    let text = opt_str(args, "theme").unwrap_or("light");
    ThemeKind::parse(text).ok_or_else(|| ApiError::invalid(format!("Unbekanntes Thema: {text}")))
}

fn open(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&loc)?;
    let meta = rt.with_read(&loc, |backend, path| backend.stat(path))?;
    if meta.is_dir {
        return Err(ApiError::invalid(
            "Ordner lassen sich nicht als Text ansehen",
        ));
    }
    let document = if loc.is_local() {
        open_local(Path::new(&loc.path))
    } else {
        let (backend, path) = rt.resolve_live(&loc)?;
        open_backend(backend, &path)
    }
    .map_err(text_error)?;
    let name = loc.name().to_string();
    let open = OpenDoc::new(document, &name);
    open.start_background();
    let markdown = open.is_markdown();
    let status = open.doc.status();
    let language = open.detected.clone();
    let id = rt.inner.texts.insert(Arc::clone(&open));
    Ok(json!({
        "docId": id,
        "name": name,
        "location": loc.location(),
        "prefix": loc.prefix,
        "path": loc.path,
        "size": open.doc.len(),
        "encoding": status.encoding.label(),
        "encodingKey": status.encoding,
        "language": language,
        "markdown": markdown,
        "lines": status.lines,
        "complete": status.complete,
    }))
}

fn lines(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let open = doc(rt, args)?;
    let theme = theme(args)?;
    let first = u64_or(args, "first", 0);
    let count = (u64_or(args, "count", 100) as usize).clamp(1, MAX_LINES_PER_CALL);
    let session = open.session(opt_str(args, "language"));
    let styled = session.highlight(&*open.doc, first, count, theme)?;
    let status = open.doc.status();
    let lines: Vec<Value> = styled
        .iter()
        .enumerate()
        .map(|(index, line)| styled_line_json(Some(first + index as u64), line))
        .collect();
    Ok(json!({
        "first": first,
        "language": session.language(),
        "lines": lines,
        "known": status.lines,
        "complete": status.complete,
        "colors": colors_json(&session.highlighter().theme_colors(theme)),
    }))
}

/// The search of `text.find` (and of the content search): mode, case, word.
pub(crate) fn text_query(args: &Value) -> Result<ContentQuery, ApiError> {
    let pattern = str_arg(args, "pattern")?;
    let mut query = ContentQuery::text(pattern);
    if let Some(mode) = opt_str(args, "mode") {
        query.mode = TextMode::parse(mode)
            .ok_or_else(|| ApiError::invalid(format!("Unbekannter Suchmodus: {mode}")))?;
    }
    query.match_case = pick_bool(args, "matchCase", "match_case", false);
    query.whole_word = pick_bool(args, "wholeWord", "whole_word", false);
    query.dot_matches_newline = pick_bool(args, "dotMatchesNewline", "dot_matches_newline", false);
    Ok(query)
}

/// A flag under its camelCase name or the core's snake_case name.
pub(crate) fn pick_bool(args: &Value, camel: &str, snake: &str, default: bool) -> bool {
    args.get(camel)
        .or_else(|| args.get(snake))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn find_in(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let open = doc(rt, args)?;
    let request = FindRequest {
        query: text_query(args)?,
        from_line: u64_or(args, "fromLine", 0),
        from_column: u64_or(args, "fromColumn", 0) as usize,
        backward: bool_or(args, "backward", false),
        wrap: bool_or(args, "wrap", true),
    };
    Ok(Found::json(find(&*open.doc, &request)?))
}

fn markdown(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let theme = theme(args)?;
    let text = match opt_str(args, "docId") {
        Some(id) => rt.inner.texts.get(id)?.whole_text()?,
        None => str_arg(args, "text")?.to_string(),
    };
    Ok(markdown_json(&parse_markdown(
        &text,
        &shared_highlighter(),
        theme,
    )))
}

fn languages() -> Value {
    Value::Array(
        shared_highlighter()
            .languages()
            .into_iter()
            .map(|language| json!({ "name": language.name, "extensions": language.extensions }))
            .collect(),
    )
}

#[cfg(test)]
#[path = "text_view_tests.rs"]
mod tests;
