use super::prelude::*;
use super::*;

impl App {
    /// Whether the current view can accept dropped files — a local folder, or a
    /// remote folder (files are uploaded via the backend).
    pub(in crate::app) fn drop_target(&self) -> Option<String> {
        if self.root_path.is_empty() {
            None
        } else if self.remote.is_some() || is_local_style(&self.root_path) {
            Some(self.root_path.clone())
        } else {
            None
        }
    }

    /// Handle files dropped onto the window from the OS (Explorer, desktop, …).
    /// They land in the current folder — copy by default, move with Shift held.
    pub(in crate::app) fn handle_os_drop(&mut self, ctx: &egui::Context) {
        let (paths, shift) = ctx.input(|i| {
            let p: Vec<String> = i
                .raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            (p, i.modifiers.shift)
        });
        if paths.is_empty() {
            return;
        }
        // Remote view → upload the dropped files into the current remote folder.
        if let Some(rs) = &self.remote {
            self.start_remote_upload(paths, rs.backend.clone(), self.root_path.clone());
            return;
        }
        let dest = match self.drop_target() {
            Some(p) => PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)),
            None => {
                self.error_msg = Some("Ablegen nur in einem lokalen Ordner möglich.".to_string());
                return;
            }
        };
        let n = paths.len();
        self.copy_paths_into(paths, dest, shift);
        self.notice = Some((
            format!(
                "📥 {} Element(e) werden {}…",
                n,
                if shift { "verschoben" } else { "kopiert" }
            ),
            std::time::Instant::now(),
        ));
    }

    /// Which tab a screen point drops onto — a tab header, or (in split) a
    /// pane. None if over neither.
    pub(in crate::app) fn drop_target_tab(&self, p: egui::Pos2) -> Option<usize> {
        if let Some((i, _)) = self.tab_header_rects.iter().find(|(_, r)| r.contains(p)) {
            return Some(*i);
        }
        if let Some((i, _)) = self.pane_rects.iter().find(|(_, r)| r.contains(p)) {
            return Some(*i);
        }
        None
    }

    /// Drop the dragged files into tab `t`'s folder. Handles every combination
    /// of local/remote source and target: local→local copy/move, local→remote
    /// upload, remote→local download. Remote→remote isn't supported yet.
    pub(in crate::app) fn drop_files_into_tab(&mut self, t: usize, move_files: bool) {
        // Target backend: Some(handle) if the target tab is a remote view.
        let (dest_str, tgt_backend) = if t == self.active_tab {
            (
                self.root_path.clone(),
                self.remote.as_ref().map(|rs| rs.backend.clone()),
            )
        } else {
            match self.tabs.get(t) {
                Some(x) => (
                    x.root_path.clone(),
                    x.remote.as_ref().map(|rs| rs.backend.clone()),
                ),
                None => return,
            }
        };
        if dest_str.is_empty() {
            return;
        }
        let dest_fwd = dest_str.trim_end_matches('/').to_string();
        let files: Vec<String> = std::mem::take(&mut self.drag_files)
            .into_iter()
            .filter(|p| p.rsplit_once('/').map(|(par, _)| par) != Some(dest_fwd.as_str()))
            .collect();
        let src_backend = self.drag_src.take();
        if files.is_empty() {
            self.notice = Some((
                "Dateien sind bereits im Ziel-Ordner.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = files.len();
        match (src_backend, tgt_backend) {
            // local → local
            (None, None) => {
                if !is_local_style(&dest_fwd) {
                    self.error_msg = Some("Ziel ist kein lokaler Ordner.".to_string());
                    return;
                }
                let dest = PathBuf::from(dest_fwd.replace('/', std::path::MAIN_SEPARATOR_STR));
                self.copy_paths_into(files, dest, move_files);
                self.notice = Some((
                    format!("{} Element(e) werden kopiert…", n),
                    std::time::Instant::now(),
                ));
            }
            // local → remote (upload)
            (None, Some(be)) => {
                self.start_remote_upload(files, be, dest_fwd);
            }
            // remote → local (download)
            (Some(be), None) => {
                if !is_local_style(&dest_fwd) {
                    self.error_msg = Some("Ziel ist kein lokaler Ordner.".to_string());
                    return;
                }
                self.start_remote_download(be, files, dest_fwd);
            }
            // remote → remote
            // remote → remote (cross-backend: download to temp, then upload)
            (Some(src), Some(tgt)) => {
                self.start_remote_to_remote(src, files, tgt, dest_fwd);
            }
        }
    }

    /// Copy remote `files` into another remote folder. When source and target
    /// are the SAME connection (`Arc::ptr_eq`), copy SERVER-LOCALLY through the
    /// backend (instant with the agent — no down+up through a temp; falls back to
    /// SFTP streaming on a plain connection). Cross-connection still streams each
    /// through a temp file. Off the UI thread; reuses the transfer channel.
    pub(in crate::app) fn start_remote_to_remote(
        &mut self,
        src: crate::vfs::BackendHandle,
        files: Vec<String>,
        tgt: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits eine Übertragung…".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = files.len();
        let same_server = std::sync::Arc::ptr_eq(&src, &tgt);
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("remote-to-remote".into())
            .spawn(move || {
                let mut copied = 0u64;
                let mut errors = Vec::new();
                for p in &files {
                    let name = p
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("datei");
                    let dest = format!("{}/{}", dest_root.trim_end_matches('/'), name);
                    let r = if same_server {
                        // No temp round-trip: copy in place on the server.
                        copy_remote_tree(&*tgt, p, &dest)
                    } else {
                        let tmp = open_temp_path(name);
                        let r = download_to(&*src, p, &tmp)
                            .and_then(|_| upload_file(&*tgt, &tmp, &dest))
                            .map(|_| ());
                        cleanup_temp_copy(&tmp);
                        r.map_err(std::io::Error::other)
                    };
                    match r {
                        Ok(_) => copied += 1,
                        Err(e) => errors.push(format!("{}: {}", name, e)),
                    }
                }
                let _ = tx.send((copied, errors));
            })
            .ok();
        self.upload_rx = Some(rx);
        let how = if same_server {
            "Remote→Remote, serverseitig"
        } else {
            "Remote→Remote"
        };
        self.notice = Some((
            format!("⇄ Übertrage {} Element(e) ({})…", n, how),
            std::time::Instant::now(),
        ));
    }

    /// Download remote `files` into a local folder, off the UI thread (reuses
    /// the upload result channel for the completion notice).
    pub(in crate::app) fn start_remote_download(
        &mut self,
        backend: crate::vfs::BackendHandle,
        files: Vec<String>,
        dest_local: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits eine Übertragung…".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = files.len();
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("remote-download-multi".into())
            .spawn(move || {
                let mut copied = 0u64;
                let mut errors = Vec::new();
                for p in &files {
                    let name = p
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("datei");
                    let dest = std::path::Path::new(&dest_local).join(name);
                    // download_node handles folders (bulk get_tree / recursive)
                    // as well as files.
                    match download_node(&*backend, p, &dest) {
                        Ok(_) => copied += 1,
                        Err(e) => errors.push(format!("{}: {}", name, e)),
                    }
                }
                let _ = tx.send((copied, errors));
            })
            .ok();
        self.upload_rx = Some(rx);
        self.notice = Some((
            format!("⬇ Lade {} Element(e) herunter…", n),
            std::time::Instant::now(),
        ));
    }

    /// Drive an active internal file drag each frame: paint a cursor chip,
    /// route a drop onto another tab/pane, and (Windows) hand the drag off to
    /// Explorer once the pointer leaves the window.
    pub(in crate::app) fn handle_file_drag(&mut self, ctx: &egui::Context) {
        if !self.drag_active {
            return;
        }
        let (down, released, pos, shift) = ctx.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.any_released(),
                i.pointer.latest_pos(),
                i.modifiers.shift,
            )
        });

        // Drag OUT to Explorer (Windows): once the pointer leaves the window
        // while still dragging, hand the files to the OS drag loop (blocks until
        // the drop completes), then refresh in case it was a move.
        #[cfg(windows)]
        if down && !self.drag_out_started {
            if let Some(p) = pos {
                if !ctx.screen_rect().contains(p) {
                    self.drag_out_started = true;
                    self.drag_active = false;
                    let files = std::mem::take(&mut self.drag_files);
                    let mut cleanup_after_drag = false;
                    // Remote source → materialize to temp copies first (Explorer
                    // needs real local paths). May briefly block on the download.
                    let files = if let Some(be) = self.drag_src.take() {
                        cleanup_after_drag = true;
                        files
                            .iter()
                            .filter_map(|p| {
                                let name = p
                                    .trim_end_matches('/')
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or("datei");
                                download_to(&*be, p, &open_temp_path(name)).ok()
                            })
                            .collect()
                    } else {
                        files
                    };
                    crate::dragout::drag_out(&files);
                    if cleanup_after_drag {
                        for f in &files {
                            cleanup_temp_copy(Path::new(f));
                        }
                    }
                    self.rescan();
                    return;
                }
            }
        }

        if down {
            // Floating chip near the cursor.
            if let Some(p) = pos {
                let n = self.drag_files.len();
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("file_drag_chip"),
                ));
                let text = format!(
                    "📄 {} Element(e){}",
                    n,
                    if shift { " — verschieben" } else { "" }
                );
                let galley =
                    painter.layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
                let pad = egui::vec2(8.0, 4.0);
                let origin = p + egui::vec2(14.0, 8.0);
                let rect = egui::Rect::from_min_size(origin, galley.size() + pad * 2.0);
                painter.rect_filled(rect, 4.0, Color32::from_rgb(40, 90, 140));
                painter.galley(origin + pad, galley, Color32::WHITE);
            }
            ctx.request_repaint();
            return;
        }

        // Released inside the window → route to a target tab/pane.
        if released {
            if let Some(t) = pos.and_then(|p| self.drop_target_tab(p)) {
                if t != self.drag_source_tab {
                    self.drop_files_into_tab(t, shift);
                }
            }
            self.drag_active = false;
            self.drag_files.clear();
            self.drag_src = None;
        }
    }

    // ─── In-app folder picker (#17) ─────────────────────────────────────
}
