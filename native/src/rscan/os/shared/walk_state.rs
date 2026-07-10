use super::budget::ScanBudget;
use super::{ext_of, join, BATCH, MAX_ERRORS_TRACKED, PROGRESS_MS};
use crate::scanner::ScanMessage;
use crate::types::{FileEntry, ScanProgress};
use crate::vfs::VfsMeta;
use crossbeam_channel::Sender;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub(super) struct WalkState {
    tx: Sender<ScanMessage>,
    cancel: Arc<AtomicBool>,
    start: Instant,
    budget: ScanBudget,
    batch: Vec<FileEntry>,
    failed: Vec<(String, String)>,
    scanned: u64,
    bytes: u64,
    errors: u64,
    last_progress: Instant,
    output_open: bool,
}

impl WalkState {
    pub(super) fn new(tx: Sender<ScanMessage>, cancel: Arc<AtomicBool>, start: Instant) -> Self {
        Self {
            tx,
            cancel,
            start,
            budget: ScanBudget::default(),
            batch: Vec::with_capacity(BATCH),
            failed: Vec::new(),
            scanned: 0,
            bytes: 0,
            errors: 0,
            last_progress: Instant::now(),
            output_open: true,
        }
    }

    pub(super) fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || !self.output_open
    }

    pub(super) fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub(super) fn send_root(&mut self, entry: FileEntry) -> bool {
        self.send(ScanMessage::Entries(vec![entry]))
    }

    pub(super) fn listing_failed(&mut self, directory: &str, error: impl ToString) {
        self.errors = self.errors.saturating_add(1);
        self.record_failure(directory, format!("list_dir: {}", error.to_string()));
    }

    pub(super) fn process_listing(
        &mut self,
        directory: &str,
        depth: u32,
        entries: Vec<VfsMeta>,
        descend: bool,
        next: &mut Vec<(String, u32)>,
    ) -> bool {
        if let Err(limit) = self.budget.preflight_entries(entries.len()) {
            self.terminal_error(
                directory,
                format!("remote scan stopped because its {limit} was reached"),
            );
            return false;
        }
        if let Err(error) = validate_listing(directory, &entries) {
            self.terminal_error(directory, error.to_string());
            return false;
        }

        let parent: Arc<str> = Arc::from(directory);
        for metadata in entries {
            if self.stopped() {
                return false;
            }
            let extension = ext_of(&metadata.name, metadata.is_dir);
            let retained_text = directory
                .len()
                .saturating_add(joined_path_len(directory, &metadata.name))
                .saturating_add(metadata.name.len())
                .saturating_add(extension.len());
            if let Err(limit) = self.budget.claim(retained_text, depth) {
                self.terminal_error(
                    directory,
                    format!(
                        "remote scan stopped because its {limit} was reached at child {}",
                        diagnostic_preview(&metadata.name)
                    ),
                );
                return false;
            }

            let path = join(directory, &metadata.name);
            let recurse = descend && metadata.is_dir && !metadata.is_symlink;
            let size = metadata.size;
            let is_dir = metadata.is_dir;
            let entry = FileEntry {
                path: Arc::from(path.as_str()),
                parent: parent.clone(),
                name: Arc::from(metadata.name.as_str()),
                ext: Arc::from(extension.as_str()),
                size,
                mtime_ms: metadata.mtime_ms,
                btime_ms: metadata.btime_ms,
                is_dir,
                is_symlink: metadata.is_symlink,
                hidden: metadata.hidden,
                system: metadata.system,
                depth,
                id: metadata.id.as_deref().map(Arc::from),
            };
            self.scanned = self.scanned.saturating_add(1);
            if !is_dir {
                self.bytes = self.bytes.saturating_add(size);
            }
            if recurse {
                next.push((path, depth.saturating_add(1)));
            }
            self.batch.push(entry);
            if self.batch.len() >= BATCH && !self.flush_batch() {
                return false;
            }
        }
        true
    }

    pub(super) fn maybe_progress(&mut self, current_path: &str) -> bool {
        if self.last_progress.elapsed().as_millis() <= PROGRESS_MS {
            return true;
        }
        self.last_progress = Instant::now();
        self.send(ScanMessage::Progress(self.progress(current_path)))
    }

    pub(super) fn terminal_error(&mut self, path: &str, detail: String) {
        if self.stopped() {
            return;
        }
        let detail = bounded_text(&detail);
        self.errors = self.errors.saturating_add(1);
        self.record_failure(path, detail.clone());
        if self.flush_batch() {
            let _ = self.send(ScanMessage::Error(detail));
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(super) fn finish(mut self) {
        if !self.output_open {
            return;
        }
        if !self.flush_batch() {
            return;
        }
        if !self.failed.is_empty() {
            let failed = std::mem::take(&mut self.failed);
            if !self.send(ScanMessage::FailedPaths(failed)) {
                return;
            }
        }
        let progress = self.progress("");
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

    fn progress(&self, current_path: &str) -> ScanProgress {
        ScanProgress {
            scanned: self.scanned,
            bytes: self.bytes,
            errors: self.errors,
            elapsed_ms: self.start.elapsed().as_millis() as u64,
            current_path: current_path.to_string(),
        }
    }

    fn record_failure(&mut self, path: &str, detail: String) {
        if self.failed.len() < MAX_ERRORS_TRACKED {
            self.failed
                .push((bounded_text(path), bounded_text(&detail)));
        }
    }
}

pub(super) fn validate_listing(directory: &str, entries: &[VfsMeta]) -> io::Result<()> {
    let mut names = HashSet::with_capacity(entries.len());
    for entry in entries {
        if crate::vfs::validate_child_name(&entry.name).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "backend returned unsafe child name in {}: {}",
                    diagnostic_preview(directory),
                    diagnostic_preview(&entry.name)
                ),
            ));
        }
        if !names.insert(entry.name.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "backend returned duplicate child name in {}: {}",
                    diagnostic_preview(directory),
                    diagnostic_preview(&entry.name)
                ),
            ));
        }
    }
    Ok(())
}

fn joined_path_len(parent: &str, name: &str) -> usize {
    parent
        .len()
        .saturating_add(usize::from(!parent.ends_with('/')))
        .saturating_add(name.len())
}

pub(super) fn diagnostic_preview(text: &str) -> String {
    format!("{:?}", bounded_text(text))
}

pub(super) fn bounded_text(text: &str) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 4096;
    if text.len() <= MAX_DIAGNOSTIC_BYTES {
        return text.to_string();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

pub(super) fn retained_text_bytes(parent: &str, path: &str, name: &str, extension: &str) -> usize {
    parent
        .len()
        .saturating_add(path.len())
        .saturating_add(name.len())
        .saturating_add(extension.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn validates_every_child_before_a_listing_is_emitted() {
        let duplicate = vec![
            VfsMeta {
                name: "same".into(),
                ..Default::default()
            },
            VfsMeta {
                name: "same".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            validate_listing("/root", &duplicate).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        for name in ["", ".", "..", "nested/name", r"nested\name", "nul\0name"] {
            let listed = vec![VfsMeta {
                name: name.into(),
                ..Default::default()
            }];
            assert!(
                validate_listing("/root", &listed).is_err(),
                "accepted unsafe child {name:?}"
            );
        }
    }

    #[test]
    fn downstream_disconnect_sets_the_shared_cancellation_flag() {
        let (tx, rx) = unbounded();
        drop(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut state = WalkState::new(tx, cancel.clone(), Instant::now());
        let root = FileEntry {
            path: Arc::from("/root"),
            parent: Arc::from("/"),
            name: Arc::from("root"),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: true,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 0,
            id: None,
        };
        assert!(!state.send_root(root));
        assert!(cancel.load(Ordering::Relaxed));
        assert!(state.stopped());
    }
}
