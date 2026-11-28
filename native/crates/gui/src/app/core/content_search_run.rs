//! A running „In Dateien suchen“: `vfs::content_search` on a background
//! thread, on the backend of the searched place, so the search runs where
//! the files are (local cores, SSH agent, Share host, otherwise reading
//! through the backend). Events stream into the result tree; the cancel flag
//! stops it.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};

use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary};

use super::content_search_results::{RecordState, SearchResults};
use super::text_tools::TextPlace;

/// Events taken from one search per frame, so a flood of hits cannot stall
/// the window (the rest follows in the next frames).
const EVENTS_PER_FRAME: usize = 4096;

pub(in crate::app) struct SearchRun {
    pub(in crate::app) id: u64,
    events: Receiver<ContentEvent>,
    done: Receiver<Result<ContentSummary, String>>,
    cancel: Arc<AtomicBool>,
    finished: Option<Result<ContentSummary, String>>,
}

impl SearchRun {
    /// Starts the search of `root` on `place`; `Err` when the thread cannot start.
    pub(in crate::app) fn start(
        id: u64,
        place: &TextPlace,
        root: String,
        query: ContentQuery,
    ) -> Result<Self, String> {
        let (event_tx, events) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let backend = place.backend_for(&root);
        std::thread::Builder::new()
            .name("content-search".into())
            .spawn(move || {
                let result = crate::vfs::content_search(
                    backend.as_ref(),
                    &root,
                    &query,
                    event_tx,
                    flag.as_ref(),
                )
                .map_err(|error| error.to_string());
                let _ = done_tx.send(result);
            })
            .map_err(|error| format!("Die Suche konnte nicht starten: {error}"))?;
        Ok(Self {
            id,
            events,
            done,
            cancel,
            finished: None,
        })
    }

    pub(in crate::app) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(in crate::app) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Moves waiting events into `results`; `true` once the search ended and
    /// every event was taken (the record then has its final state).
    pub(in crate::app) fn drain(&mut self, results: &mut SearchResults) -> bool {
        if self.finished.is_none() {
            match self.done.try_recv() {
                Ok(result) => self.finished = Some(result),
                Err(TryRecvError::Disconnected) => {
                    self.finished = Some(Err("Die Suche wurde unerwartet beendet.".to_string()))
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        for _ in 0..EVENTS_PER_FRAME {
            match self.events.try_recv() {
                Ok(event) => results.apply(self.id, event),
                Err(_) => break,
            }
        }
        if !self.events.is_empty() {
            return false;
        }
        let Some(result) = self.finished.take() else {
            return false;
        };
        let state = match result {
            Ok(summary) => RecordState::Done(summary),
            Err(_) if self.cancelled() => RecordState::Cancelled,
            Err(error) => RecordState::Failed(error),
        };
        results.finish(self.id, state);
        true
    }
}

impl Drop for SearchRun {
    fn drop(&mut self) {
        self.cancel();
    }
}
