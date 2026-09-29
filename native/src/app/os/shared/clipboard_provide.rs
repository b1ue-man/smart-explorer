//! "Für andere Programme bereitstellen" (Windows): programs that only take
//! real files (browser uploads, messengers) get the remote selection as a
//! file clipboard. The engine downloads it into a private temp folder of this
//! session with progress in the transfer list; afterwards the folder's
//! entries become CF_HDROP. This replaces the former implicit download on
//! Ctrl+C, and the app is never blocked while it runs.
use super::prelude::*;
use super::transfer_center::TransferPurpose;
use super::transfer_route::{paste_job, TransferPlace};
use super::transfer_selection::SelectionIssue;
use super::*;
use crate::app::shared_platform_helpers::ClipboardEffect;

impl App {
    pub(in crate::app) fn provide_for_other_programs(&mut self) {
        if !clipboard_file_ops_supported() {
            self.error_msg =
                Some("Datei-Zwischenablage ist auf dieser Plattform nicht verfügbar.".to_string());
            return;
        }
        if self.remote.is_none() {
            self.notice = Some((
                "Lokale Dateien liegen mit Strg+C bereits als Dateien in der Zwischenablage"
                    .to_string(),
                Instant::now(),
            ));
            return;
        }
        let view = match self.view_selection(false) {
            Ok(view) => view,
            Err(SelectionIssue::Empty(hint)) => {
                self.notice = Some((hint, Instant::now()));
                return;
            }
            Err(SelectionIssue::Invalid(error)) => {
                self.error_msg = Some(error);
                return;
            }
        };
        // `open_temp_path` creates a private folder and names a path in it;
        // the folder receives the download, the path identifies the folder
        // for `cleanup_temp_copy`.
        let temp = match open_temp_path("bereitgestellt") {
            Ok(temp) => temp,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Temporärer Ordner für die Bereitstellung nicht verfügbar: {error}"
                ));
                return;
            }
        };
        let Some(folder) = temp.parent() else {
            return;
        };
        let target_dir = folder
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let count = view.selection.count();
        let job = match paste_job(
            &view.selection,
            &TransferPlace::local(),
            &target_dir,
            CopyMode::Copy,
        ) {
            Ok(job) => job,
            Err(error) => {
                cleanup_temp_copy(&temp);
                self.error_msg = Some(error);
                return;
            }
        };
        let purpose = TransferPurpose::Provide {
            temp: temp.clone(),
            sequence: virtual_clipboard_sequence(),
        };
        if self.submit_job_for(job, purpose) {
            self.notice = Some((
                format!(
                    "⬇ Lade {count} Element(e) für andere Programme herunter — Fortschritt unter „Übertragungen“"
                ),
                Instant::now(),
            ));
        } else {
            cleanup_temp_copy(&temp);
        }
    }

    /// The download of a provide request ended: publish its files unless it
    /// was abandoned or the clipboard changed since the request.
    pub(in crate::app) fn publish_provided(
        &mut self,
        temp: &Path,
        sequence: Option<u32>,
        abandoned: bool,
        with_errors: bool,
    ) {
        let paths = if abandoned {
            Vec::new()
        } else {
            provided_paths(temp)
        };
        if paths.is_empty() {
            cleanup_temp_copy(temp);
            if !abandoned {
                self.notice = Some((
                    "⚠ Nichts heruntergeladen — die Zwischenablage bleibt unverändert".to_string(),
                    Instant::now(),
                ));
            }
            return;
        }
        let published = match sequence {
            Some(sequence) => {
                write_clipboard_files_if_sequence(&paths, ClipboardEffect::Copy, sequence)
                    .map(|written| written.is_some())
            }
            None => write_clipboard_files(&paths, ClipboardEffect::Copy).map(|()| true),
        };
        match published {
            Ok(true) => {
                // The OS clipboard now holds these files, not our entry.
                self.clip = None;
                let text = if with_errors {
                    format!(
                        "⚠ {} Element(e) liegen für andere Programme bereit, mit Fehlern — Details unter „Übertragungen“",
                        paths.len()
                    )
                } else {
                    format!(
                        "✓ {} Element(e) liegen für andere Programme bereit — dort mit Strg+V einfügen",
                        paths.len()
                    )
                };
                self.notice = Some((text, Instant::now()));
            }
            Ok(false) => {
                cleanup_temp_copy(temp);
                self.notice = Some((
                    "⚠ Die Zwischenablage wurde inzwischen geändert — bitte erneut „Für andere Programme bereitstellen“ wählen"
                        .to_string(),
                    Instant::now(),
                ));
            }
            // The adapter can fail after publishing (for example on closing
            // the clipboard); the files stay for the session in that case.
            Err(error) => self.error_msg = Some(format!("Zwischenablage: {error}")),
        }
    }
}

/// The entries of the provide folder as Windows file-clipboard paths.
fn provided_paths(temp: &Path) -> Vec<String> {
    let Some(folder) = temp.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_string_lossy().into_owned())
        .collect();
    paths.sort();
    paths
}
