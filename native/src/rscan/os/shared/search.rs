use super::budget::ScanBudget;
use super::walk_state::{bounded_text, diagnostic_preview, retained_text_bytes};
use super::{ext_of, join, report_spawn_failure, BATCH, PROGRESS_MS};
use crate::agent_proto::{SearchSpec, ValidatedRelativePath};
use crate::scanner::{ScanHandle, ScanMessage};
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::{BackendHandle, SearchHit};
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SEARCH_BACKLOG: usize = 256;
const SEARCH_POLL: Duration = Duration::from_millis(50);

/// Run a server-side recursive search and present its relative matches through
/// the same flat scan stream used by client-side navigation.
pub fn start_search_backend(
    backend: BackendHandle,
    root: String,
    spec: SearchSpec,
    tx: Sender<ScanMessage>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let failure_tx = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("rscan-search".into())
        .spawn(move || run_search(backend, root, spec, tx, worker_cancel))
    {
        report_spawn_failure(&failure_tx, &cancel, "remote search", error);
    }
    ScanHandle { cancel }
}

fn run_search(
    backend: BackendHandle,
    root: String,
    spec: SearchSpec,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
) {
    let mut relay = SearchRelay::new(root.clone(), tx, cancel.clone());
    let (hit_tx, hit_rx) = search_channel();
    let worker_root = root;
    let worker_cancel = cancel.clone();
    let worker = match std::thread::Builder::new()
        .name("agent-search".into())
        .spawn(move || backend.search(&worker_root, &spec, hit_tx, &worker_cancel))
    {
        Ok(worker) => worker,
        Err(error) => {
            relay.terminal_error(format!("server-search worker could not start: {error}"));
            relay.finish();
            return;
        }
    };

    while !relay.stopped() {
        match hit_rx.recv_timeout(SEARCH_POLL) {
            Ok(hit) => {
                if !relay.accept_hit(hit) {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if !relay.maybe_progress() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Dropping the bounded receiver releases a backend blocked in `send` after
    // cancellation or a downstream disconnect.
    drop(hit_rx);
    match worker.join() {
        Ok(Err(error)) if relay.can_report_worker_failure() => {
            relay.terminal_error(format!("server-side search failed: {error}"));
        }
        Ok(Ok(false)) if relay.can_report_worker_failure() => {
            relay.terminal_error(
                "backend advertised server-side search but reported it unsupported".to_string(),
            );
        }
        Err(_) if relay.can_report_worker_failure() => {
            relay.terminal_error("server-search worker panicked".to_string());
        }
        _ => {}
    }
    relay.finish();
}

fn search_channel() -> (Sender<SearchHit>, Receiver<SearchHit>) {
    crossbeam_channel::bounded(SEARCH_BACKLOG)
}

struct SearchRelay {
    root: String,
    root_arc: Arc<str>,
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    start: Instant,
    last_progress: Instant,
    budget: ScanBudget,
    seen: HashSet<Arc<str>>,
    batch: Vec<FileEntry>,
    scanned: u64,
    bytes: u64,
    errors: u64,
    output_open: bool,
    terminal_failure: bool,
}

impl SearchRelay {
    fn new(root: String, tx: Sender<ScanMessage>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            root_arc: Arc::from(root.as_str()),
            root,
            tx,
            cancel,
            start: Instant::now(),
            last_progress: Instant::now(),
            budget: ScanBudget::default(),
            seen: HashSet::new(),
            batch: Vec::with_capacity(BATCH),
            scanned: 0,
            bytes: 0,
            errors: 0,
            output_open: true,
            terminal_failure: false,
        }
    }

    fn stopped(&self) -> bool {
        !self.output_open || self.cancel.load(Ordering::Relaxed)
    }

    fn can_report_worker_failure(&self) -> bool {
        self.output_open && !self.terminal_failure && !self.cancel.load(Ordering::Relaxed)
    }

    fn accept_hit(&mut self, hit: SearchHit) -> bool {
        let relative = match validate_search_relative(&hit.rel) {
            Ok(relative) => relative,
            Err(error) => {
                self.terminal_error(format!(
                    "server-side search returned unsafe relative path {}: {error}",
                    diagnostic_preview(&hit.rel)
                ));
                return false;
            }
        };
        let relative_text: Arc<str> = Arc::from(relative.path.as_str());
        if !self.seen.insert(relative_text.clone()) {
            self.terminal_error(format!(
                "server-side search returned duplicate relative path {}",
                diagnostic_preview(relative.path.as_str())
            ));
            return false;
        }

        let path = join(&self.root, relative.path.as_str());
        let base = relative
            .path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or(relative.path.as_str());
        let extension = ext_of(base, hit.is_dir);
        let retained_text =
            retained_text_bytes(&self.root, &path, relative.path.as_str(), &extension);
        if let Err(limit) = self.budget.claim(retained_text, relative.depth) {
            self.terminal_error(format!(
                "server-side search stopped because its {limit} was reached at {}",
                diagnostic_preview(relative.path.as_str())
            ));
            return false;
        }

        self.scanned = self.scanned.saturating_add(1);
        if !hit.is_dir {
            self.bytes = self.bytes.saturating_add(hit.size);
        }
        self.batch.push(FileEntry {
            path: Arc::from(path.as_str()),
            parent: self.root_arc.clone(),
            name: relative_text,
            ext: Arc::from(extension.as_str()),
            size: hit.size,
            mtime_ms: hit.mtime_ms,
            btime_ms: 0,
            is_dir: hit.is_dir,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 1,
            id: None,
        });
        self.batch.len() < BATCH || self.flush_batch()
    }

    fn maybe_progress(&mut self) -> bool {
        if self.last_progress.elapsed().as_millis() <= PROGRESS_MS {
            return true;
        }
        self.last_progress = Instant::now();
        let progress = self.progress(self.root.clone());
        self.send(ScanMessage::Progress(progress))
    }

    fn terminal_error(&mut self, detail: String) {
        if !self.output_open || self.terminal_failure {
            return;
        }
        let detail = bounded_text(&detail);
        self.terminal_failure = true;
        self.errors = self.errors.saturating_add(1);
        if self.flush_batch() {
            let _ = self.send(ScanMessage::Error(detail));
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn finish(mut self) {
        if !self.output_open || !self.flush_batch() {
            return;
        }
        let progress = self.progress(String::new());
        let _ = self.send(ScanMessage::Done(progress));
    }

    fn flush_batch(&mut self) -> bool {
        if self.batch.is_empty() {
            return self.output_open;
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(BATCH));
        self.send(ScanMessage::Entries(batch))
    }

    fn send(&mut self, message: ScanMessage) -> bool {
        if !self.output_open {
            return false;
        }
        if self.tx.send(message).is_ok() {
            true
        } else {
            self.output_open = false;
            self.cancel.store(true, Ordering::Relaxed);
            false
        }
    }

    fn progress(&self, current_path: String) -> ScanProgress {
        ScanProgress {
            scanned: self.scanned,
            bytes: self.bytes,
            errors: self.errors,
            elapsed_ms: self.start.elapsed().as_millis() as u64,
            current_path,
        }
    }
}

struct SearchRelative {
    path: ValidatedRelativePath,
    depth: u32,
}

fn validate_search_relative(raw: &str) -> io::Result<SearchRelative> {
    let path = ValidatedRelativePath::parse(raw)?;
    let depth = u32::try_from(path.depth()).unwrap_or(u32::MAX);
    Ok(SearchRelative { path, depth })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::TrySendError;

    fn hit(relative: &str) -> SearchHit {
        SearchHit {
            rel: relative.to_string(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
        }
    }

    #[test]
    fn rejects_unsafe_search_hit_relative_paths() {
        for relative in [
            "",
            "/absolute",
            "../escape",
            "safe/../../escape",
            "safe//file",
            r"safe\file",
            "safe/C:escape",
            "safe/file:stream",
        ] {
            assert!(
                validate_search_relative(relative).is_err(),
                "accepted unsafe relative search hit {relative:?}"
            );
        }
    }

    #[test]
    fn internal_search_relay_channel_is_bounded() {
        let (sender, _receiver) = search_channel();
        for index in 0..SEARCH_BACKLOG {
            sender.try_send(hit(&format!("hit-{index}"))).unwrap();
        }
        assert!(matches!(
            sender.try_send(hit("overflow")),
            Err(TrySendError::Full(_))
        ));
    }

    #[test]
    fn duplicate_search_hits_are_terminal_errors() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut relay = SearchRelay::new("/root".into(), tx, cancel.clone());
        assert!(relay.accept_hit(hit("same")));
        assert!(!relay.accept_hit(hit("same")));
        assert!(cancel.load(Ordering::Relaxed));
        relay.finish();
        assert!(matches!(rx.recv().unwrap(), ScanMessage::Entries(_)));
        assert!(matches!(rx.recv().unwrap(), ScanMessage::Error(_)));
        assert!(matches!(rx.recv().unwrap(), ScanMessage::Done(_)));
    }

    #[test]
    fn search_relay_applies_depth_limit_to_valid_relative_paths() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut relay = SearchRelay::new("/root".into(), tx, cancel);
        let too_deep = std::iter::repeat_n("d", 513).collect::<Vec<_>>().join("/");
        assert!(validate_search_relative(&too_deep).is_ok());
        assert!(!relay.accept_hit(hit(&too_deep)));
        relay.finish();
        assert!(matches!(rx.recv().unwrap(), ScanMessage::Error(_)));
        assert!(matches!(rx.recv().unwrap(), ScanMessage::Done(_)));
    }

    #[test]
    fn search_relay_cancels_when_its_consumer_disconnects() {
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut relay = SearchRelay::new("/root".into(), tx, cancel.clone());
        let progress = relay.progress("/root".into());
        assert!(!relay.send(ScanMessage::Progress(progress)));
        assert!(cancel.load(Ordering::Relaxed));
    }
}
