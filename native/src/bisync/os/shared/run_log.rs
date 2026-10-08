//! Live log of a saved job's runs: one appendable text file per job,
//! `<sync data>/job-logs/<id>.log`, readable while a run writes it. Every
//! runner (background service, desktop window, Android, terminal) writes
//! through the same engine, so one file shows all of a job's runs.
//!
//! Logging never changes a run's outcome: a log that cannot be opened or
//! written is skipped. The file rotates to `<id>.log.1` at
//! `JOB_LOG_ROTATE_BYTES` (disk bound; a verbose full run over 100 000
//! entries writes about 10 MB). Each written line also marks the job as
//! active in this process, which the background service uses as its
//! progress signal (a run that only scans and compares is not stalled).
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use super::completion::{ApplySink, CompletedAction};
use super::omissions::OmissionKind;
use super::run_types::RunStop;

/// Size at which the current log moves to `<id>.log.1`.
pub const JOB_LOG_ROTATE_BYTES: u64 = 8 * 1024 * 1024;
/// Most bytes one read returns (a viewer's first page and each refresh).
pub const JOB_LOG_READ_BYTES: usize = 1024 * 1024;
const MAX_ID_LEN: usize = 128;

static ACTIVITY: Mutex<BTreeMap<String, i64>> = Mutex::new(BTreeMap::new());

thread_local! {
    static CURRENT: RefCell<Option<Arc<RunLog>>> = const { RefCell::new(None) };
}

/// The open log of one job.
pub struct RunLog {
    job_id: String,
    path: PathBuf,
    verbose: bool,
    file: Mutex<Option<(File, u64)>>,
}

impl RunLog {
    /// Appends one line: local time, a short category and the text. Line
    /// breaks inside `text` are kept on the same line.
    pub fn line(&self, tag: &str, text: &str) {
        let now = chrono::Local::now();
        let text = text.replace(['\r', '\n'], " ⏎ ");
        let line = format!("{} {tag:<10} {text}\n", now.format("%Y-%m-%d %H:%M:%S%.3f"));
        mark_active(&self.job_id, now.timestamp());
        let mut slot = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(_, len)| *len >= JOB_LOG_ROTATE_BYTES)
        {
            *slot = None;
            let _ = rotate(&self.path);
        }
        if slot.is_none() {
            *slot = open_append(&self.path).ok();
        }
        if let Some((file, len)) = slot.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                *len = len.saturating_add(line.len() as u64);
            } else {
                *slot = None;
            }
        }
    }

    /// Unchanged entries are written one per line (the job's log switch).
    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}

fn logs_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir().join("job-logs")
}

/// Job ids are `[A-Za-z0-9_-]` (`syncjobs` sanitizes them); anything else
/// never becomes a file name.
fn safe_id(id: &str) -> Option<&str> {
    (!id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then_some(id)
}

/// Where the job's current log lives.
pub fn job_log_path(job_id: &str) -> Option<PathBuf> {
    safe_id(job_id).map(|id| logs_dir().join(format!("{id}.log")))
}

fn verbose_flag(job_id: &str) -> Option<PathBuf> {
    safe_id(job_id).map(|id| logs_dir().join(format!("{id}.verbose")))
}

fn open_append(path: &PathBuf) -> io::Result<(File, u64)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let len = file.metadata()?.len();
    Ok((file, len))
}

fn rotate(path: &PathBuf) -> io::Result<()> {
    let mut previous = path.clone().into_os_string();
    previous.push(".1");
    match std::fs::remove_file(&previous) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::rename(path, previous)
}

/// Opens the job's log for one run; `None` when the id is unusable or the
/// file cannot be opened (the run continues without a log).
pub fn open_job_log(job_id: &str) -> Option<Arc<RunLog>> {
    let path = job_log_path(job_id)?;
    let mut opened = open_append(&path).ok()?;
    if opened.1 >= JOB_LOG_ROTATE_BYTES {
        drop(opened);
        rotate(&path).ok()?;
        opened = open_append(&path).ok()?;
    }
    Some(Arc::new(RunLog {
        job_id: job_id.to_string(),
        path,
        verbose: job_log_verbose(job_id),
        file: Mutex::new(Some(opened)),
    }))
}

/// One line outside a run (trigger, connection failure, result).
pub fn job_log_line(job_id: &str, tag: &str, text: &str) {
    if let Some(log) = open_job_log(job_id) {
        log.line(tag, text);
    }
}

/// Whether unchanged entries are logged one by one.
pub fn job_log_verbose(job_id: &str) -> bool {
    verbose_flag(job_id).is_some_and(|path| path.is_file())
}

pub fn set_job_log_verbose(job_id: &str, verbose: bool) -> io::Result<()> {
    let path = verbose_flag(job_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe sync job id"))?;
    if verbose {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        File::create(path).map(|_| ())
    } else {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

fn mark_active(job_id: &str, now: i64) {
    let mut activity = ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner);
    activity.insert(job_id.to_string(), now);
}

/// Unix seconds of the job's last log line written by this process.
pub fn last_activity(job_id: &str) -> Option<i64> {
    ACTIVITY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(job_id)
        .copied()
}

/// A piece of the log for a viewer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogChunk {
    /// Complete lines only.
    pub text: String,
    /// Offset to pass to the next read.
    pub next: u64,
    /// Current file size.
    pub size: u64,
    /// The file was rotated or truncated since the given offset; `text`
    /// starts at the new file's last page.
    pub restarted: bool,
}

/// Reads complete lines from `from` (`None`: the last page). A missing log
/// is an empty chunk.
pub fn read_job_log(job_id: &str, from: Option<u64>) -> io::Result<LogChunk> {
    let path = job_log_path(job_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe sync job id"))?;
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LogChunk {
                restarted: from.is_some_and(|offset| offset > 0),
                ..LogChunk::default()
            })
        }
        Err(error) => return Err(error),
    };
    let size = file.metadata()?.len();
    let restarted = from.is_some_and(|offset| offset > size);
    let tail = size.saturating_sub(JOB_LOG_READ_BYTES as u64);
    let (start, align) = match from {
        Some(offset) if offset <= size => (offset, false),
        _ => (tail, tail > 0),
    };
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(JOB_LOG_READ_BYTES as u64)
        .read_to_end(&mut bytes)?;
    let begin = if align {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |at| at + 1)
    } else {
        0
    };
    let end = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(begin, |at| (at + 1).max(begin));
    Ok(LogChunk {
        text: String::from_utf8_lossy(&bytes[begin..end]).into_owned(),
        next: start + end as u64,
        size,
        restarted,
    })
}

/// Makes `log` the current thread's run log until the guard drops.
pub(super) struct CurrentLog {
    previous: Option<Arc<RunLog>>,
}

pub(super) fn enter(log: Option<Arc<RunLog>>) -> CurrentLog {
    let previous = CURRENT.with(|current| current.replace(log));
    CurrentLog { previous }
}

impl Drop for CurrentLog {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

/// The run log of the run executing on this thread (walk contexts copy it
/// to their worker threads).
pub(super) fn current() -> Option<Arc<RunLog>> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Writes to the current run's log, if any.
pub(super) fn note(tag: &str, text: &str) {
    if let Some(log) = current() {
        log.line(tag, text);
    }
}

/// Logs every apply event and forwards it to the run's own observer.
pub(super) struct LoggingSink<'a> {
    pub(super) log: Arc<RunLog>,
    pub(super) inner: Option<&'a dyn ApplySink>,
}

impl ApplySink for LoggingSink<'_> {
    fn completed(&self, action: CompletedAction) {
        self.log
            .line("Aktion", &super::run_log_lines::completed_text(&action));
        if let Some(inner) = self.inner {
            inner.completed(action);
        }
    }

    fn should_stop(&self) -> bool {
        self.inner.is_some_and(|inner| inner.should_stop())
    }

    fn omitted(&self, rel: &str, kind: OmissionKind) {
        self.log.line(
            "Ausgelassen",
            &format!("{rel} ({kind:?}); bleibt geschützt"),
        );
        if let Some(inner) = self.inner {
            inner.omitted(rel, kind);
        }
    }

    fn deferred(&self, rel: &str, reason: &str) {
        self.log.line(
            "Verschoben",
            &format!("{rel}: {reason}; der nächste Lauf übernimmt es"),
        );
        if let Some(inner) = self.inner {
            inner.deferred(rel, reason);
        }
    }

    fn stopped(&self, stop: RunStop) {
        self.log
            .line("Gestoppt", &format!("Übernahme beendet: {stop:?}"));
        if let Some(inner) = self.inner {
            inner.stopped(stop);
        }
    }
}
