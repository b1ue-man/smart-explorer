use super::clipboard_state::PreparationStamp;
use super::prelude::*;
use super::*;
use crate::app::shared_platform_helpers::{ClipboardEffect, ClipboardVirtualFile};

/// Owns only paths produced by the remote clipboard downloader. A dropped
/// receiver or stale result must release these session files. Ambiguous publish
/// errors retain them: CF_HDROP may already refer to them despite a close error.
pub(in crate::app) struct PreparedTempClipboard {
    paths: Vec<String>,
}

impl PreparedTempClipboard {
    pub fn new(paths: Vec<String>) -> Self {
        Self { paths }
    }

    fn retain_for_session(mut self) {
        // Published files may still be consumed by Explorer. Existing session
        // cleanup owns their lifetime after clipboard publication succeeds.
        self.paths.clear();
    }
}

impl Drop for PreparedTempClipboard {
    fn drop(&mut self) {
        for path in &self.paths {
            cleanup_temp_copy(Path::new(path));
        }
    }
}

impl App {
    pub(in crate::app) fn cancel_clipboard_preparation(&mut self) {
        self.clipboard_preparation.clear();
        self.clip_prepare_rx = None;
        self.clip_download_rx = None;
    }

    pub(in crate::app) fn begin_clipboard_preparation(&mut self) -> Option<PreparationStamp> {
        self.cancel_clipboard_preparation();
        self.virtual_clip = None;
        let Some(sequence) = virtual_clipboard_sequence() else {
            self.error_msg = Some("Zwischenablage: Änderungsstand nicht lesbar.".to_string());
            return None;
        };
        match self.clipboard_preparation.begin(sequence) {
            Ok(stamp) => Some(stamp),
            Err(error) => {
                self.error_msg = Some(error);
                None
            }
        }
    }

    fn refresh_clipboard_preparation(&mut self) {
        if let Some(stamp) = self.clipboard_preparation.pending() {
            if virtual_clipboard_sequence() != Some(stamp.sequence) {
                self.cancel_clipboard_preparation();
                self.notice = Some((
                    "Zwischenablage geändert — ältere Vorbereitung verworfen.".to_string(),
                    Instant::now(),
                ));
            }
        }
    }

    pub(in crate::app) fn clipboard_paste_is_pending(&mut self) -> bool {
        self.refresh_clipboard_preparation();
        if self.clipboard_preparation.pending().is_none() {
            return false;
        }
        self.notice = Some((
            "Zwischenablage wird vorbereitet — bitte danach erneut einfügen.".to_string(),
            Instant::now(),
        ));
        true
    }

    pub(in crate::app) fn drain_clip_prepare(&mut self) {
        self.refresh_clipboard_preparation();
        let prepared = match self.clip_prepare_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.cancel_clipboard_preparation();
                self.error_msg =
                    Some("Gefilterte Zwischenablage wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.clip_prepare_rx = None;
        if self.clipboard_preparation.pending() != Some(prepared.stamp) {
            return;
        }
        if !self.clipboard_preparation.accepts(prepared.stamp, virtual_clipboard_sequence()) {
            self.cancel_clipboard_preparation();
            return;
        }
        let files = match prepared.result {
            Ok(files) => files,
            Err(error) => {
                self.cancel_clipboard_preparation();
                self.error_msg = Some(error);
                return;
            }
        };
        if files.is_empty() {
            self.cancel_clipboard_preparation();
            self.notice = Some((
                "Keine Dateien entsprechen dem aktiven Filter".to_string(),
                Instant::now(),
            ));
            return;
        }
        let pairs = files.iter().map(|f| (f.abs.clone(), f.rel.clone())).collect();
        let n = files.len();
        let published = set_virtual_clipboard_if_sequence(files, prepared.stamp.sequence);
        // OLE may dispatch messages. A newer own preparation must keep its
        // receiver/state even if it was started during the platform call.
        if self.clipboard_preparation.pending() != Some(prepared.stamp) {
            return;
        }
        self.cancel_clipboard_preparation();
        match published {
            Ok(Some(seq)) if virtual_clipboard_sequence() == Some(seq) => {
                self.virtual_clip = Some((seq, pairs));
                self.notice = Some((
                    format!("✓ {n} gefilterte Datei(en) kopiert — Einfügen erhält die Ordnerstruktur"),
                    Instant::now(),
                ));
            }
            Ok(_) => {}
            Err(error) => self.error_msg = Some(format!("Zwischenablage: {error}")),
        }
    }

    pub(in crate::app) fn drain_clip_download(&mut self) {
        self.refresh_clipboard_preparation();
        let prepared = match self.clip_download_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.cancel_clipboard_preparation();
                self.error_msg = Some("Zwischenablage: Download-Worker wurde beendet".to_string());
                return;
            }
        };
        self.clip_download_rx = None;
        if self.clipboard_preparation.pending() != Some(prepared.stamp) {
            return;
        }
        if !self.clipboard_preparation.accepts(prepared.stamp, virtual_clipboard_sequence()) {
            self.cancel_clipboard_preparation();
            return;
        }
        self.clipboard_preparation.clear();
        let local = match prepared.result {
            Ok(local) if !local.paths.is_empty() => local,
            Ok(_) => {
                self.error_msg = Some("Zwischenablage: keine Dateien vorbereitet".to_string());
                return;
            }
            Err(error) => {
                self.error_msg = Some(format!("Zwischenablage: {error}"));
                return;
            }
        };
        match write_clipboard_files_if_sequence(
            &local.paths,
            ClipboardEffect::Copy,
            prepared.stamp.sequence,
        ) {
            Ok(Some(_)) => {
                self.virtual_clip = None;
                self.notice = Some((
                    format!("✓ {} Element(e) kopiert — in Explorer einfügbar (Ctrl+V)", local.paths.len()),
                    Instant::now(),
                ));
                local.retain_for_session();
            }
            Ok(None) => {}
            Err(error) => {
                // The adapter can fail after publishing CF_HDROP (for example
                // CloseClipboard). Do not delete paths another app may read.
                local.retain_for_session();
                self.error_msg = Some(format!("Zwischenablage: {error}"));
            }
        }
    }
}

pub(in crate::app) fn prepare_filtered_clipboard(
    seeds: Vec<FileEntry>,
    filter: FilterDef,
    prefix: String,
) -> Result<Vec<ClipboardVirtualFile>, String> {
    let cf = CompiledFilter::compile(&filter);
    let mut out = Vec::new();
    for e in &seeds {
        if e.is_dir && !e.is_symlink {
            let base = format!("{}/", e.parent.trim_end_matches('/'));
            let collected = crate::scanner::collect_recursive(
                &PathBuf::from(e.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                false,
                e.depth + 1,
                &std::sync::atomic::AtomicBool::new(false),
            );
            if !collected.is_complete() {
                let first = collected.issues.first()
                    .map(|issue| format!("{}: {}", issue.path, issue.detail))
                    .unwrap_or_else(|| "unvollständige Ordnererfassung".to_string());
                let total = collected.issues.len() as u64 + collected.suppressed_issues;
                return Err(format!(
                    "Gefilterte Zwischenablage konnte nicht vollständig erstellt werden ({total} Fehler): {first}"
                ));
            }
            for s in collected.entries {
                if !s.is_dir && cf.matches(&s, &prefix) {
                    out.push(ClipboardVirtualFile {
                        abs: s.path.replace('/', "\\"),
                        rel: s.path.strip_prefix(base.as_str()).unwrap_or(s.name.as_ref()).to_string(),
                        size: s.size,
                        mtime_ms: s.mtime_ms,
                    });
                    if out.len() >= 1_000_000 {
                        return Err("Gefilterte Zwischenablage überschreitet das Limit von 1.000.000 Dateien.".to_string());
                    }
                }
            }
        } else {
            out.push(ClipboardVirtualFile {
                abs: e.path.replace('/', "\\"),
                rel: e.name.to_string(),
                size: e.size,
                mtime_ms: e.mtime_ms,
            });
        }
    }
    Ok(out)
}
