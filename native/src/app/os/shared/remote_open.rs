use super::prelude::*;
use super::*;
use crate::app::shared_platform_helpers::ClipboardEffect;

#[path = "remote_zip.rs"]
mod zip;

fn should_cleanup_missing_temp(mtime_ms: i64, process_finished: bool, dirty: bool) -> bool {
    mtime_ms == 0 && process_finished && !dirty
}

#[cfg(test)]
#[path = "remote_open_tests.rs"]
mod tests;

impl App {
    /// Open a file in its associated app (`OpenMode::Default`) or via the native
    /// Windows "Open with…" chooser (`OpenMode::With`). Local files launch
    /// directly; a remote file is downloaded to a temp copy off the UI thread,
    /// then launched when ready (so it "just works" on SFTP/FTP/WebDAV/Drive too,
    /// and the temp copy is edit-watched so saves upload back).
    pub(in crate::app) fn open_file(
        &mut self,
        path: String,
        name: String,
        id: Option<String>,
        mode: OpenMode,
    ) {
        let rs = match &self.remote {
            Some(rs) => rs,
            None => {
                // A local .zip opens INSIDE the explorer (browse as a folder);
                // everything else hands off to the OS.
                match mode {
                    OpenMode::Default if is_zip_name(&name) => self.open_zip(&path),
                    OpenMode::Default => self.open_path(&path),
                    OpenMode::With => self.open_with_path(&path),
                }
                return;
            }
        };
        let backend = rs.backend.clone();

        // Download to a local temp copy, watch it for saves, and launch the OS
        // default editor. `download_name` gives Google-Docs files the right
        // extension (.docx/…) so the editor opens them correctly.
        let local_name = backend.download_name(&path, &name);
        if self.remote_edits.len() >= 50 {
            self.error_msg = Some(
                "Zu viele offene Remote-Dateien: bitte einige Editor-Fenster schliessen."
                    .to_string(),
            );
            return;
        }
        let dest = match open_temp_path(&local_name) {
            Ok(dest) => dest,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Temporärer Pfad für „{name}“ konnte nicht angelegt werden: {error}"
                ));
                return;
            }
        };
        self.remote_edits.retain(|e| e.temp != dest);
        self.remote_edits.push(RemoteEdit {
            temp: dest.clone(),
            backend: backend.clone(),
            remote_path: path.clone(),
            name: name.clone(),
            baseline_mtime: i64::MAX, // real value set once downloaded
            seen_mtime: 0,
            remote_known_mtime: 0, // captured after download (below)
            dirty: false,
            uploading: false,
            process: None,
        });
        if let Err(error) = preserve_session_temp(&self.remote_edits, false) {
            self.remote_edits.retain(|edit| edit.temp != dest);
            cleanup_temp_copy(&dest);
            self.error_msg = Some(format!(
                "Remote-Datei kann ohne Wiederherstellungsmanifest nicht sicher geöffnet werden: {error}"
            ));
            return;
        }
        let (tx, rx) = unbounded();
        let dest_t = dest.clone();
        let spawn = std::thread::Builder::new()
            .name("remote-open".into())
            .spawn(move || {
                // Capture the remote's mtime at download time so save-back can
                // detect a concurrent remote change before overwriting.
                let res = download_to_id(&*backend, &path, id.as_deref(), &dest_t).map(|p| {
                    let rm = backend.stat(&path).map(|m| m.mtime_ms).unwrap_or(0);
                    (p, rm)
                });
                let _ = tx.send(res);
            });
        match spawn {
            Ok(_) => {
                self.file_open_rx.push((rx, mode, dest));
                self.notice = Some((
                    format!("⬇ Öffne „{}“ (Speichern landet auf dem Remote)…", name),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.remote_edits.retain(|edit| edit.temp != dest);
                cleanup_temp_copy(&dest);
                self.error_msg = Some(format!(
                    "Remote-Datei konnte nicht geöffnet werden: {error}"
                ));
            }
        }
    }

    /// Open the file at `idx` via the native "Open with…" chooser (downloading a
    /// remote file to a temp copy first). Folders are ignored.
    pub(in crate::app) fn open_with_entry(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        let e = &self.entries[idx];
        if e.is_dir {
            return;
        }
        let (path, name) = (e.path.to_string(), e.name.to_string());
        let id = e.id.as_ref().map(|s| s.to_string());
        self.open_file(path, name, id, OpenMode::With);
    }

    /// Launch any remote files that finished downloading to temp.
    pub(in crate::app) fn drain_file_open(&mut self) {
        if self.file_open_rx.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        let mut to_open = Vec::new();
        for (rx, mode, temp) in std::mem::take(&mut self.file_open_rx) {
            match rx.try_recv() {
                Ok(Ok((p, remote_mtime))) => to_open.push((p, remote_mtime, mode, temp)),
                Ok(Err(e)) => {
                    self.remote_edits.retain(|edit| edit.temp != temp);
                    cleanup_temp_copy(&temp);
                    self.error_msg = Some(format!("Datei oeffnen: {}", e));
                }
                Err(crossbeam_channel::TryRecvError::Empty) => pending.push((rx, mode, temp)),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.remote_edits.retain(|edit| edit.temp != temp);
                    cleanup_temp_copy(&temp);
                    self.error_msg = Some("Remote-Datei-Download wurde unerwartet beendet.".into());
                }
            }
        }
        self.file_open_rx = pending;
        for (p, remote_mtime, mode, _temp) in to_open {
            // Baseline the edit-watch to the freshly downloaded content so we
            // don't immediately re-upload it; only the user's saves count. Record
            // the remote's mtime so save-back can detect a concurrent change.
            let pb = PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR));
            let m = file_mtime_ms(&pb);
            let process = self.launch_for_edit(&p, mode);
            if let Some(e) = self.remote_edits.iter_mut().find(|e| e.temp == pb) {
                e.baseline_mtime = m;
                e.seen_mtime = m;
                e.remote_known_mtime = remote_mtime;
                e.dirty = false;
                e.process = process;
            }
        }
    }

    /// Poll temp-mode edit copies; re-upload to the remote when one is saved
    /// (mtime advances and is stable for one ~1.5s cycle = a completed write).
    pub(in crate::app) fn poll_remote_edits(&mut self) {
        if self.remote_edits.is_empty() {
            return;
        }
        if self.last_edit_poll.elapsed() < std::time::Duration::from_millis(1500) {
            return;
        }
        self.last_edit_poll = std::time::Instant::now();
        let mut launch: Vec<(PathBuf, crate::vfs::BackendHandle, String, String, i64)> = Vec::new();
        let mut cleanup_done: Vec<PathBuf> = Vec::new();
        let mut manifest_changed = false;
        for e in self.remote_edits.iter_mut().filter(|e| !e.uploading) {
            let m = file_mtime_ms(&e.temp);
            if m == 0 {
                let process_finished = e.process.as_ref().map(|p| p.is_finished()).unwrap_or(false);
                if should_cleanup_missing_temp(m, process_finished, e.dirty) {
                    cleanup_done.push(e.temp.clone());
                }
                continue;
            }
            // Sentinel: first time we actually see the file (e.g. after CfAPI
            // hydration), just baseline it — don't treat the initial content as
            // an edit to re-upload.
            if e.baseline_mtime == i64::MAX {
                e.baseline_mtime = m;
                e.seen_mtime = m;
                e.dirty = false;
                manifest_changed = true;
                continue;
            }
            if m == e.baseline_mtime {
                continue;
            }
            e.dirty = true;
            manifest_changed = true;
            if m == e.seen_mtime {
                e.uploading = true;
                e.baseline_mtime = m;
                launch.push((
                    e.temp.clone(),
                    e.backend.clone(),
                    e.remote_path.clone(),
                    e.name.clone(),
                    e.remote_known_mtime,
                ));
            } else {
                e.seen_mtime = m;
            }
        }
        if !cleanup_done.is_empty() {
            for temp in &cleanup_done {
                cleanup_temp_copy(temp);
            }
            self.remote_edits
                .retain(|e| !cleanup_done.iter().any(|temp| temp == &e.temp));
            manifest_changed = true;
        }
        if manifest_changed && !self.remote_edits.is_empty() {
            if let Err(error) = preserve_session_temp(&self.remote_edits, false) {
                self.error_msg = Some(format!("Remote-Wiederherstellung aktualisieren: {error}"));
            }
        }
        for (temp, be, remote, name, known) in launch {
            let (tx, rx) = unbounded();
            let expected_temp = temp.clone();
            let spawn = std::thread::Builder::new()
                .name("remote-edit-save".into())
                .spawn(move || {
                    // Conflict guard: if the remote advanced past what we last
                    // knew, it changed underneath us — don't overwrite.
                    let current = be.stat(&remote).map(|m| m.mtime_ms).unwrap_or(0);
                    let res = if known != 0 && current > known {
                        SaveResult::Conflict(current)
                    } else {
                        match upload_file(&*be, &temp, &remote) {
                            Ok(()) => {
                                let nm = be.stat(&remote).map(|m| m.mtime_ms).unwrap_or(0);
                                SaveResult::Ok(nm)
                            }
                            Err(e) => SaveResult::Failed(e),
                        }
                    };
                    let _ = tx.send((temp, res));
                });
            match spawn {
                Ok(_) => {
                    self.edit_save_rx.push((rx, expected_temp));
                    self.notice = Some((
                        format!("↑ Speichere „{}“ auf dem Remote…", name),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    if let Some(edit) = self
                        .remote_edits
                        .iter_mut()
                        .find(|edit| edit.temp == expected_temp)
                    {
                        edit.uploading = false;
                        edit.dirty = true;
                        edit.baseline_mtime = 0;
                    }
                    self.error_msg = Some(format!(
                        "Remote-Speichern konnte nicht gestartet werden: {error}"
                    ));
                }
            }
        }
    }

    pub(in crate::app) fn drain_edit_saves(&mut self) {
        if self.edit_save_rx.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        let mut manifest_changed = false;
        for (rx, expected_temp) in std::mem::take(&mut self.edit_save_rx) {
            match rx.try_recv() {
                Ok((temp, res)) => {
                    manifest_changed = true;
                    if let Some(e) = self.remote_edits.iter_mut().find(|e| e.temp == temp) {
                        e.uploading = false;
                        match res {
                            SaveResult::Ok(new_remote) => {
                                e.remote_known_mtime = new_remote;
                                e.dirty = false;
                                self.notice = Some((
                                    format!("✓ „{}“ auf dem Remote gespeichert", e.name),
                                    std::time::Instant::now(),
                                ));
                            }
                            SaveResult::Conflict(remote_mtime) => {
                                // Remote changed since we opened it. We did NOT
                                // overwrite. Adopt the remote mtime as the new
                                // baseline so the next deliberate save wins, and
                                // keep the local edit as-is.
                                e.remote_known_mtime = remote_mtime;
                                e.baseline_mtime = file_mtime_ms(&temp);
                                e.seen_mtime = e.baseline_mtime;
                                e.dirty = true;
                                self.error_msg = Some(format!(
                                    "Konflikt „{}“: Die Remote-Datei wurde inzwischen geändert — \
                                     deine lokale Änderung wurde NICHT hochgeladen (kein Überschreiben). \
                                     Öffne die Datei erneut für die Remote-Version, oder speichere \
                                     erneut, um deine Version durchzusetzen.",
                                    e.name
                                ));
                            }
                            SaveResult::Failed(err) => {
                                e.baseline_mtime = 0; // let a later save retry
                                e.dirty = true;
                                self.error_msg =
                                    Some(format!("Remote speichern „{}“: {}", e.name, err));
                            }
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => pending.push((rx, expected_temp)),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    manifest_changed = true;
                    if let Some(edit) = self
                        .remote_edits
                        .iter_mut()
                        .find(|edit| edit.temp == expected_temp)
                    {
                        edit.uploading = false;
                        edit.dirty = true;
                        edit.baseline_mtime = 0;
                    }
                    self.error_msg = Some(format!(
                        "Remote-Speichern „{}“ wurde unerwartet beendet; die lokale Datei bleibt zur Wiederholung erhalten.",
                        expected_temp.display()
                    ));
                }
            }
        }
        self.edit_save_rx = pending;
        if manifest_changed && !self.remote_edits.is_empty() {
            if let Err(error) = preserve_session_temp(&self.remote_edits, false) {
                self.error_msg = Some(format!("Remote-Wiederherstellung aktualisieren: {error}"));
            }
        }
    }

    /// Upload local `paths` (files and/or folders, recursively) into the remote
    /// folder `dest_root` via `backend`, off the UI thread. Used by Ctrl+V and
    /// drag-drop into a remote view.
    pub(in crate::app) fn start_remote_upload(
        &mut self,
        paths: Vec<String>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Upload — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = paths.len();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let spawn = std::thread::Builder::new()
            .name("remote-upload".into())
            .spawn(move || {
                upload_paths_progress(&*backend, &paths, &dest_root, &tx, &worker_cancel);
            });
        match spawn {
            Ok(worker) => {
                self.upload_rx = Some(rx);
                self.transfer_cancel = Some(cancel);
                self.transfer_worker = Some(worker);
                self.transfer_progress = Some(TransferProgress::new(
                    TransferKind::Upload,
                    "Lade hoch",
                    n as u64,
                    0,
                ));
                self.notice = Some((
                    format!("⬆ Lade {} Element(e) hoch…", n),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.upload_rx = None;
                self.transfer_progress = None;
                self.transfer_cancel = None;
                self.transfer_worker = None;
                self.error_msg = Some(format!(
                    "Remote-Upload konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    /// Once selected remote files have downloaded to temp, put them on the
    /// Windows clipboard as CF_HDROP so they paste into Explorer.
    pub(in crate::app) fn drain_clip_download(&mut self) {
        let result = match self.clip_download_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.clip_download_rx = None;
                self.error_msg = Some("Zwischenablage: Download-Worker wurde beendet".to_string());
                return;
            }
        };
        self.clip_download_rx = None;
        let local = match result {
            Ok(local) if !local.is_empty() => local,
            Ok(_) => {
                self.error_msg = Some("Zwischenablage: keine Dateien vorbereitet".to_string());
                return;
            }
            Err(error) => {
                self.error_msg = Some(format!("Zwischenablage: {error}"));
                return;
            }
        };
        match write_clipboard_files(&local, ClipboardEffect::Copy) {
            Ok(_) => {
                self.virtual_clip = None;
                self.notice = Some((
                    format!(
                        "✓ {} Element(e) kopiert - in Explorer einfuegbar (Ctrl+V)",
                        local.len()
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => self.error_msg = Some(format!("Zwischenablage: {}", e)),
        }
    }

    // ─── Peer file sharing (#21) ─────────────────────────────────────────
}
