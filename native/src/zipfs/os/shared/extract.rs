//! Controlled extraction for callers that need progress and cancellation
//! (the Android facade): into an existing folder, never replacing a file,
//! zip-slip safe like `extract_all`. Per-entry failures are reported and the
//! remaining entries are still extracted.
use super::names::enclosed_name;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use zip::result::ZipError;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Default)]
pub struct ExtractReport {
    pub files: u64,
    pub bytes: u64,
    /// Entries not written: `(archive path, reason)`.
    pub skipped: Vec<(String, String)>,
    pub canceled: bool,
}

/// Skip reason for an entry that cannot be opened.
fn open_failure(error: ZipError) -> String {
    match error {
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
            "verschlüsselt (Passwort nötig)".to_string()
        }
        ZipError::UnsupportedArchive(reason) => format!("nicht unterstützt ({reason})"),
        other => format!("nicht lesbar: {other}"),
    }
}

fn native(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Extracts every regular file of `zip_path` below `dest` (which must exist).
/// Existing files are never replaced; links inside the archive are skipped.
/// Unix modes of the archive are never read or applied: a new file gets the
/// process default, so no SUID/SGID bit can arrive from an entry.
///
/// Names, links and folders are decided in archive order on this thread;
/// the files are written by parallel lanes (`extract_lanes`) while this
/// thread reports their progress. Like every copy, an extracted file is not
/// forced to disk one by one (G4 MZ2): a crash leaves at most partial files,
/// as an interrupted extraction always could.
pub fn extract_all_controlled(
    zip_path: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&ExtractProgress),
) -> io::Result<ExtractReport> {
    if !std::fs::symlink_metadata(dest)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Entpack-Ziel ist kein Ordner",
        ));
    }
    let archive = super::extract_lanes::open(&native(zip_path))?;
    let mut state = ExtractProgress::default();
    let mut report = ExtractReport::default();
    let mut skipped: Vec<(usize, String, String)> = Vec::new();
    let mut jobs = Vec::new();
    {
        let mut archive = archive.clone();
        for index in 0..archive.len() {
            // Metadata only (no decryption or decompressor): an entry that
            // cannot be opened is skipped by its lane, not fatal here.
            let Ok(entry) = archive.by_index_raw(index) else {
                continue;
            };
            if entry.is_file() && !entry.is_symlink() && enclosed_name(&entry).is_some() {
                state.files_total += 1;
                state.bytes_total = state.bytes_total.saturating_add(entry.size());
            }
        }
        progress(&state);
        for index in 0..archive.len() {
            if cancel.load(Ordering::Acquire) {
                report.canceled = true;
                break;
            }
            let entry = match archive.by_index_raw(index) {
                Ok(entry) => entry,
                Err(error) => {
                    let name = format!("Eintrag {}", index + 1);
                    skipped.push((index, name, open_failure(error)));
                    continue;
                }
            };
            let name = entry.name().map_or_else(
                |_| format!("Eintrag {}", index + 1),
                |name| name.into_owned(),
            );
            let Some(relative) = enclosed_name(&entry) else {
                skipped.push((index, name, "unsicherer Pfad im Archiv".to_string()));
                continue;
            };
            let target = dest.join(relative);
            if entry.is_dir() {
                if let Err(error) = std::fs::create_dir_all(&target) {
                    skipped.push((index, name, error.to_string()));
                }
                continue;
            }
            if entry.is_symlink() {
                skipped.push((index, name, "Links werden nicht entpackt".to_string()));
                continue;
            }
            jobs.push(super::extract_lanes::Job {
                index,
                name,
                target,
            });
        }
    }
    if !report.canceled {
        let outcome = run_lanes(&archive, jobs, cancel, &mut state, &mut progress);
        report.files = outcome.files;
        report.bytes = outcome.bytes;
        report.canceled = outcome.canceled;
        skipped.extend(outcome.skipped);
    }
    skipped.sort_by_key(|(index, _, _)| *index);
    report.skipped = skipped
        .into_iter()
        .map(|(_, name, reason)| (name, reason))
        .collect();
    Ok(report)
}

/// What a lane tells the reporting thread.
enum Event {
    Bytes(u64),
    Written(u64),
    Skipped(usize, String, String),
    Canceled,
}

#[derive(Default)]
struct LaneOutcome {
    files: u64,
    bytes: u64,
    canceled: bool,
    skipped: Vec<(usize, String, String)>,
}

/// Writes the file entries in parallel lanes and reports their progress on
/// this thread, in the order the lanes produce it.
fn run_lanes(
    archive: &super::entry::Archive,
    jobs: Vec<super::extract_lanes::Job>,
    cancel: &AtomicBool,
    state: &mut ExtractProgress,
    progress: &mut impl FnMut(&ExtractProgress),
) -> LaneOutcome {
    let mut outcome = LaneOutcome::default();
    std::thread::scope(|scope| {
        let (events, received) = std::sync::mpsc::channel::<Event>();
        for lane in super::extract_lanes::lanes(jobs) {
            let mut archive = archive.clone();
            let events = events.clone();
            scope.spawn(move || {
                for job in lane {
                    if cancel.load(Ordering::Acquire) {
                        let _ = events.send(Event::Canceled);
                        return;
                    }
                    let mut entry = match archive.by_index(job.index) {
                        Ok(entry) => entry,
                        Err(error) => {
                            let _ = events.send(Event::Skipped(
                                job.index,
                                job.name,
                                open_failure(error),
                            ));
                            continue;
                        }
                    };
                    let sent = |bytes: u64| {
                        let _ = events.send(Event::Bytes(bytes));
                    };
                    let event = match write_new(&mut entry, &job.target, cancel, sent) {
                        Ok(Some(bytes)) => Event::Written(bytes),
                        Ok(None) => Event::Canceled,
                        Err(error) => Event::Skipped(job.index, job.name, error.to_string()),
                    };
                    let stop = matches!(event, Event::Canceled);
                    if events.send(event).is_err() || stop {
                        return;
                    }
                }
            });
        }
        drop(events);
        for event in received {
            match event {
                Event::Bytes(bytes) => {
                    state.bytes_done = state.bytes_done.saturating_add(bytes);
                }
                Event::Written(bytes) => {
                    outcome.files += 1;
                    outcome.bytes = outcome.bytes.saturating_add(bytes);
                    state.files_done += 1;
                }
                Event::Skipped(index, name, reason) => {
                    outcome.skipped.push((index, name, reason));
                    state.files_done += 1;
                }
                Event::Canceled => outcome.canceled = true,
            }
            progress(state);
        }
    });
    outcome
}

/// Writes one entry to a new file; `None` when canceled (the partial file is
/// removed).
fn write_new(
    entry: &mut impl Read,
    target: &Path,
    cancel: &AtomicBool,
    copied: impl FnMut(u64),
) -> io::Result<Option<u64>> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let result = super::extract_lanes::copy(entry, &mut output, cancel, copied);
    drop(output);
    if !matches!(result, Ok(Some(_))) {
        let _ = std::fs::remove_file(target);
    }
    result
}
