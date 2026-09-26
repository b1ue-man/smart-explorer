//! Controlled extraction for callers that need progress and cancellation
//! (the Android facade): into an existing folder, never replacing a file,
//! zip-slip safe like `extract_all`. Per-entry failures are reported and the
//! remaining entries are still extracted.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use zip::result::ZipError;

const CHUNK: usize = 256 * 1024;

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

fn zip_error<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
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
    let file = std::fs::File::open(native(zip_path))?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    let mut state = ExtractProgress::default();
    for index in 0..archive.len() {
        // Metadata only (no decryption or decompressor): an entry that cannot
        // be opened is skipped by the extraction pass, not fatal here.
        let Ok(entry) = archive.by_index_raw(index) else {
            continue;
        };
        if entry.is_file() && !entry.is_symlink() && entry.enclosed_name().is_some() {
            state.files_total += 1;
            state.bytes_total = state.bytes_total.saturating_add(entry.size());
        }
    }
    progress(&state);
    let mut report = ExtractReport::default();
    for index in 0..archive.len() {
        if cancel.load(Ordering::Acquire) {
            report.canceled = true;
            break;
        }
        let (name, counted) = match archive.by_index_raw(index) {
            Ok(raw) => (
                raw.name().to_string(),
                raw.is_file() && !raw.is_symlink() && raw.enclosed_name().is_some(),
            ),
            Err(error) => {
                let name = format!("Eintrag {}", index + 1);
                report.skipped.push((name, open_failure(error)));
                continue;
            }
        };
        // Encrypted entries and unsupported methods fail to open; they are
        // skipped like any other entry that cannot be written.
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(error) => {
                report.skipped.push((name, open_failure(error)));
                if counted {
                    state.files_done += 1;
                    progress(&state);
                }
                continue;
            }
        };
        let Some(relative) = entry.enclosed_name() else {
            report
                .skipped
                .push((name, "unsicherer Pfad im Archiv".to_string()));
            continue;
        };
        let target = dest.join(relative);
        if entry.is_dir() {
            if let Err(error) = std::fs::create_dir_all(&target) {
                report.skipped.push((name, error.to_string()));
            }
            continue;
        }
        if entry.is_symlink() {
            report
                .skipped
                .push((name, "Links werden nicht entpackt".to_string()));
            continue;
        }
        match write_new(&mut entry, &target, cancel, &mut state, &mut progress) {
            Ok(Some(bytes)) => {
                report.files += 1;
                report.bytes = report.bytes.saturating_add(bytes);
            }
            Ok(None) => {
                report.canceled = true;
                break;
            }
            Err(error) => report.skipped.push((name, error.to_string())),
        }
        state.files_done += 1;
        progress(&state);
    }
    Ok(report)
}

/// Writes one entry to a new file; `None` when canceled (the partial file is
/// removed).
fn write_new(
    entry: &mut impl Read,
    target: &Path,
    cancel: &AtomicBool,
    state: &mut ExtractProgress,
    progress: &mut impl FnMut(&ExtractProgress),
) -> io::Result<Option<u64>> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut written = 0u64;
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Ok(None);
        }
        let read = match entry.read(&mut buffer) {
            Ok(0) => break Ok(Some(written)),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => break Err(error),
        };
        if let Err(error) = output.write_all(&buffer[..read]) {
            break Err(error);
        }
        written += read as u64;
        state.bytes_done = state.bytes_done.saturating_add(read as u64);
        progress(state);
    };
    let result = result.and_then(|done| output.sync_all().map(|()| done));
    drop(output);
    if !matches!(result, Ok(Some(_))) {
        let _ = std::fs::remove_file(target);
    }
    result
}
