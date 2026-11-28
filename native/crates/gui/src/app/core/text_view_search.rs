//! Go to line and find in the file for a text view window: the go-to bar
//! (Strg+G) and the search (Strg+F, F3/Umschalt+F3) on a background thread
//! over the document (`text_view_find::find_next`).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::text_view_find::{find_next, FindFrom, FindOutcome};
use super::text_view_window::TextViewWindow;

impl TextViewWindow {
    /// Strg+G: goes to the typed line or keeps the bar open with the error.
    pub(in crate::app) fn apply_goto(&mut self) {
        let total = self.index_complete().then(|| self.known_lines());
        match super::text_view_find::parse_goto(&self.goto.text, total) {
            Ok(line) => {
                self.marks.clear();
                self.jump_to(line);
                self.goto.error = None;
                self.goto.open = false;
            }
            Err(error) => self.goto.error = Some(error),
        }
    }

    /// F3 / Umschalt+F3: the next or previous match from the current one (or
    /// from the marked or first visible line), on a background thread.
    pub(in crate::app) fn start_find(&mut self, forward: bool) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        if self.find.text.is_empty() {
            self.find.open = true;
            self.find.focus = true;
            return;
        }
        let matcher = match super::content_search_nav::line_regex(
            &self.find.text,
            self.find.regex,
            self.find.match_case,
        ) {
            Ok(matcher) => matcher,
            Err(error) => {
                self.find.status = Some(error);
                return;
            }
        };
        if let Some((_, cancel)) = self.find.job.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let from = match self.find.current {
            Some(hit) if forward => FindFrom {
                line: hit.line,
                byte: hit.end,
            },
            Some(hit) => FindFrom {
                line: hit.line,
                byte: hit.start,
            },
            None => FindFrom {
                line: self.cursor.unwrap_or(self.top),
                byte: 0,
            },
        };
        let known = self.known_lines();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let started = std::thread::Builder::new()
            .name("text-view-find".into())
            .spawn(move || {
                let _ = tx.send(find_next(
                    doc.as_ref(),
                    &matcher,
                    from,
                    forward,
                    known,
                    flag.as_ref(),
                ));
            });
        match started {
            Ok(_) => {
                self.find.status = Some("Sucht …".to_string());
                self.find.job = Some((rx, cancel));
            }
            Err(error) => self.find.status = Some(format!("Suche konnte nicht starten: {error}")),
        }
    }

    pub(in crate::app) fn poll_find(&mut self) {
        let Some((rx, _)) = &self.find.job else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.find.job = None;
        match result {
            Ok(FindOutcome {
                hit: Some(hit),
                wrapped,
            }) => {
                self.find.current = Some(hit);
                self.marks = vec![(hit.start, hit.end)];
                self.jump_to(hit.line);
                self.find.status = wrapped.then(|| "Am Dateiende neu begonnen.".to_string());
            }
            Ok(FindOutcome { hit: None, .. }) => {
                self.find.current = None;
                self.find.status = Some(format!("„{}“ nicht gefunden.", self.find.text));
            }
            Err(error) => self.find.status = Some(error.to_string()),
        }
    }
}
