//! Every entry a transfer did not transfer, with its reason. All of them go
//! to a log file (one JSON object per line) under the app data folder, so a
//! transfer with thousands of problems stays fully traceable; the first ones
//! also travel with the terminal message for the transfer list.
use super::super::types::TransferIssue;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Issues carried in `TransferMsg::Done` (plan: the first 100); the log file
/// holds all of them.
pub(crate) const SHOWN_ISSUES: usize = 100;
/// One message never grows beyond this (backend errors can embed whole
/// server replies); the same bound the older transfer error list used.
const MAX_MESSAGE_BYTES: usize = 4 * 1024;
/// Logs older than this are removed when a new one starts: the same horizon
/// as the app trash, long enough to look back at last month's transfers.
const LOG_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const LOG_PREFIX: &str = "transfer-";
const LOG_SUFFIX: &str = ".jsonl";

enum LogFile {
    NotYet,
    Open { file: File, path: String },
    Unavailable,
}

pub(crate) struct IssueLog {
    job_id: u64,
    dir: PathBuf,
    total: AtomicU64,
    shown: Mutex<Vec<TransferIssue>>,
    file: Mutex<LogFile>,
}

/// Where transfer logs live.
pub(crate) fn log_dir() -> PathBuf {
    crate::support_dirs::app_data_dir().join("transfer-logs")
}

impl IssueLog {
    pub(crate) fn new(job_id: u64, dir: PathBuf) -> Self {
        Self {
            job_id,
            dir,
            total: AtomicU64::new(0),
            shown: Mutex::new(Vec::new()),
            file: Mutex::new(LogFile::NotYet),
        }
    }

    /// Records one issue; `path` is empty for problems of the whole job.
    pub(crate) fn push(&self, path: &str, message: &str) {
        let message = bounded(message);
        self.total.fetch_add(1, Ordering::AcqRel);
        {
            let mut shown = self
                .shown
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if shown.len() < SHOWN_ISSUES {
                shown.push(TransferIssue {
                    path: path.to_string(),
                    message: message.clone(),
                });
            }
        }
        self.write(path, &message);
    }

    pub(crate) fn total(&self) -> u64 {
        self.total.load(Ordering::Acquire)
    }

    pub(crate) fn log_path(&self) -> Option<String> {
        match &*self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            LogFile::Open { path, .. } => Some(path.clone()),
            LogFile::NotYet | LogFile::Unavailable => None,
        }
    }

    /// The issues carried in the terminal message and their display lines.
    pub(crate) fn shown(&self) -> (Vec<TransferIssue>, Vec<String>) {
        let issues = self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let lines = issues.iter().map(display_line).collect();
        (issues, lines)
    }

    fn write(&self, path: &str, message: &str) {
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*file, LogFile::NotYet) {
            *file = match self.open() {
                Ok((opened, path)) => LogFile::Open { file: opened, path },
                Err(_) => LogFile::Unavailable,
            };
        }
        if let LogFile::Open { file: opened, .. } = &mut *file {
            let mut line = serde_json::json!({ "path": path, "message": message }).to_string();
            line.push('\n');
            if opened.write_all(line.as_bytes()).is_err() {
                // The list in the transfer entry stays complete for the first
                // issues; a log that cannot be written is not retried per line.
                *file = LogFile::Unavailable;
            }
        }
    }

    fn open(&self) -> std::io::Result<(File, String)> {
        std::fs::create_dir_all(&self.dir)?;
        prune_old_logs(&self.dir);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!(
            "{LOG_PREFIX}{stamp}-p{}-j{}{LOG_SUFFIX}",
            std::process::id(),
            self.job_id
        );
        let path = self.dir.join(name);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((file, path.to_string_lossy().into_owned()))
    }
}

pub(crate) fn display_line(issue: &TransferIssue) -> String {
    if issue.path.is_empty() {
        issue.message.clone()
    } else {
        format!("{}: {}", issue.path, issue.message)
    }
}

fn bounded(message: &str) -> String {
    if message.len() <= MAX_MESSAGE_BYTES {
        return message.to_string();
    }
    let mut end = MAX_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn prune_old_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(LOG_PREFIX) && name.ends_with(LOG_SUFFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > LOG_RETENTION);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_issue_log_writes_every_issue_as_json_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let log = IssueLog::new(7, dir.path().to_path_buf());
        assert!(log.log_path().is_none(), "no file before the first issue");
        for index in 0..(SHOWN_ISSUES + 5) {
            log.push(&format!("/src/f{index}"), "kaputt \"zitiert\"");
        }
        log.push("", "Übertragung beendet");
        let path = log.log_path().expect("log file after the first issue");
        let text = std::fs::read_to_string(path).expect("log readable");
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect();
        assert_eq!(lines.len(), SHOWN_ISSUES + 6);
        assert_eq!(lines[0]["path"], "/src/f0");
        assert_eq!(lines[0]["message"], "kaputt \"zitiert\"");
        assert_eq!(lines[SHOWN_ISSUES + 5]["path"], "");
        let (shown, display) = log.shown();
        assert_eq!(shown.len(), SHOWN_ISSUES);
        assert_eq!(display[0], "/src/f0: kaputt \"zitiert\"");
        assert_eq!(log.total(), SHOWN_ISSUES as u64 + 6);
        assert_eq!(
            bounded(&"x".repeat(10_000)).len(),
            MAX_MESSAGE_BYTES + '…'.len_utf8()
        );
    }
}
