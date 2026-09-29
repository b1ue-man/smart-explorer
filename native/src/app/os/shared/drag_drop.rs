use super::prelude::*;
use super::transfer_route::{parent_dir, TransferPlace, TransferSelection, REMOTE_MOVE_REFUSED};
use super::transfer_selection::os_paths_selection;
use super::*;

impl App {
    /// The folder the current view accepts dropped files into — any open
    /// local or remote folder.
    pub(in crate::app) fn drop_target(&self) -> Option<String> {
        if self.root_path.is_empty() {
            None
        } else if self.remote.is_some() || is_local_path(&self.root_path) {
            Some(self.root_path.clone())
        } else {
            None
        }
    }

    /// Handle files dropped onto the window from the OS (Explorer, desktop, …).
    /// They land in the current folder — copy by default, move with Shift held
    /// (local folders only). Starts at once like a paste.
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
        if self.drop_target().is_none() {
            self.error_msg = Some("Ablegen nur in einem geöffneten Ordner möglich.".to_string());
            return;
        }
        let Some((target, target_dir)) = self.tab_place(self.active_tab) else {
            return;
        };
        let Some(selection) = os_paths_selection(paths) else {
            return;
        };
        let mode = if shift {
            CopyMode::Move
        } else {
            CopyMode::Copy
        };
        self.submit_paste(&selection, &target, &target_dir, mode);
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

    /// Drop the dragged files into tab `t`'s folder: every combination of
    /// local and remote source and target runs as one engine job. Remote
    /// moves are explicitly unsupported and never silently become copies.
    pub(in crate::app) fn drop_files_into_tab(&mut self, t: usize, move_files: bool) {
        let src_backend = self.drag_src.take();
        let src_filter = self.drag_filter.take();
        let dragged = std::mem::take(&mut self.drag_files);
        let Some((target, target_dir)) = self.tab_place(t) else {
            return;
        };
        let source = match src_backend {
            Some(backend) => {
                let label = self
                    .tab_place(self.drag_source_tab)
                    .map(|(place, _)| place.label)
                    .unwrap_or_default();
                TransferPlace::remote(backend, label)
            }
            None => TransferPlace::local(),
        };
        if move_files && !(source.is_local() && target.is_local()) {
            self.error_msg = Some(REMOTE_MOVE_REFUSED.to_string());
            return;
        }
        if target.is_local() && !is_local_path(&target_dir) {
            self.error_msg = Some("Ziel ist kein lokaler Ordner.".to_string());
            return;
        }
        // Entries dropped onto their own folder change nothing; entries below
        // another dragged folder travel with it.
        let same_place = source.same_place(&target);
        let files = outermost(dragged)
            .into_iter()
            .filter(|path| {
                !same_place
                    || parent_dir(path).trim_end_matches('/') != target_dir.trim_end_matches('/')
            })
            .collect::<Vec<_>>();
        if files.is_empty() {
            self.notice = Some((
                "Dateien sind bereits im Ziel-Ordner.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Local drags carry whole folders as the file clipboard did; remote
        // drags keep the source view's filter for selected folders.
        let (base, filter) = if source.is_local() {
            (Some(parent_dir(&files[0])), None)
        } else {
            (None, src_filter)
        };
        let selection = TransferSelection::roots(source, files, base).with_filter(filter);
        let mode = if move_files {
            CopyMode::Move
        } else {
            CopyMode::Copy
        };
        self.submit_paste(&selection, &target, &target_dir, mode);
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

        // Drag OUT to the platform file manager when the adapter supports it.
        if os_drag_out_supported() && down && !self.drag_out_started {
            if let Some(p) = pos {
                if !ctx.screen_rect().contains(p) {
                    self.drag_out_started = true;
                    self.drag_active = false;
                    self.drag_out(shift);
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
            self.drag_filter = None;
        }
    }

    /// Hand the dragged entries to the platform's file manager: local files
    /// as they are, remote entries as virtual files the target reads on
    /// demand (nothing is downloaded before the drop).
    fn drag_out(&mut self, shift: bool) {
        let files = std::mem::take(&mut self.drag_files);
        let filter = self.drag_filter.take();
        let Some(backend) = self.drag_src.take() else {
            match drag_out_files(&files) {
                Ok(crate::dragout::DragOutOutcome::Dropped(
                    crate::dragout::DragOutEffect::Move,
                )) => self.rescan(),
                Ok(_) => {}
                Err(error) => self.error_msg = Some(format!("Drag-and-drop: {error}")),
            }
            return;
        };
        if shift {
            self.error_msg = Some("Remote-Verschieben wird nicht unterstützt. Bitte ohne Umschalttaste kopieren; die Quellen bleiben unverändert.".to_string());
            return;
        }
        let label = self
            .tab_place(self.drag_source_tab)
            .map(|(place, _)| place.label)
            .unwrap_or_default();
        let source = crate::transfer::SelectionSource {
            backend,
            paths: outermost(files),
            filter,
            label,
        };
        match drag_out_remote(source) {
            Ok(crate::dragout::DragOutOutcome::Dropped(crate::dragout::DragOutEffect::Move)) => {
                self.error_msg = Some("Das Ziel hat Verschieben gewählt; kopiert wurde nur der Inhalt. Die Remote-Quelldateien bleiben unverändert.".to_string());
            }
            Ok(_) => {}
            Err(error) => self.error_msg = Some(format!("Drag-and-drop: {error}")),
        }
    }
}

/// Dragged paths without those below another dragged folder (a recursive
/// view can select a folder and its contents together); each ancestor is
/// looked up once, so large selections stay linear.
fn outermost(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    let dragged: HashSet<&str> = paths
        .iter()
        .map(|path| path.trim_end_matches('/'))
        .collect();
    let below_dragged = |path: &str| {
        let mut current = path.trim_end_matches('/');
        while let Some((parent, _)) = current.rsplit_once('/') {
            if parent.is_empty() {
                return false;
            }
            if dragged.contains(parent) {
                return true;
            }
            current = parent;
        }
        false
    };
    paths
        .iter()
        .filter(|path| !below_dragged(path.as_str()))
        .cloned()
        .collect()
}
