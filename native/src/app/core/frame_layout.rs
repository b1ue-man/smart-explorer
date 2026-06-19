use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn update_layout(&mut self, ctx: &egui::Context) {
        // ─── Layout ────────────────────────────────────────────────────
        // Rebuild the Alt-overlay target list fresh each frame: clear before any
        // panel registers (tabbar renders first), repopulate during rendering.
        self.accel_targets.clear();
        egui::TopBottomPanel::top("tabbar")
            .min_height(26.0)
            .show(ctx, |ui| self.ui_tabbar(ui));

        egui::TopBottomPanel::top("toolbar")
            .min_height(32.0)
            .show(ctx, |ui| self.ui_toolbar(ui));

        // Collapsible filter section: the header is always present (so the
        // panel can be re-opened from there), the body folds away.
        egui::TopBottomPanel::top("filterbar").show(ctx, |ui| {
            let active = self.filter_is_active();
            let title = if active {
                RichText::new("🔍 Filter & Suche  ●").strong().color(Color32::from_rgb(255, 190, 90))
            } else {
                RichText::new("🔍 Filter & Suche").strong()
            };
            let header = egui::CollapsingHeader::new(title)
                .id_salt("filter_collapse")
                .open(Some(self.show_filters))
                .show(ui, |ui| self.ui_filterbar(ui));
            if header.header_response.clicked() {
                self.show_filters = !self.show_filters;
                self.save_ui_state();
            }
        });

        egui::TopBottomPanel::bottom("status")
            .min_height(22.0)
            .show(ctx, |ui| self.ui_status(ui));

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(190.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.ui_sidebar(ui));
            });

        if self.show_summary {
            egui::SidePanel::right("summary")
                .resizable(true)
                .default_width(280.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ui_summary(ui));
                });
        }

        self.ui_central(ctx);

        if self.copy_open {
            self.ui_copy_dialog(ctx);
        }
        if self.show_errors_dialog {
            self.ui_errors_dialog(ctx);
        }
        if self.rename_open.is_some() {
            self.ui_rename_dialog(ctx);
        }
        if self.show_help {
            self.ui_help_dialog(ctx);
        }
        if self.show_analytics {
            self.ui_analytics(ctx);
        }
        if self.update_ready.is_some() {
            self.ui_update_dialog(ctx);
        }
        if self.show_connect {
            self.ui_connect_dialog(ctx);
        }
        self.ui_bisync_conflicts(ctx);
        if self.merge.is_some() {
            self.ui_merge(ctx);
        }
        if self.show_sync_jobs {
            self.ui_sync_jobs(ctx);
        }
        if self.show_preview {
            self.ui_preview(ctx);
        }
        if self.show_daemon_log {
            self.ui_daemon_log(ctx);
        }
        if self.job_editor.is_some() {
            self.ui_job_editor(ctx);
        }
        if self.picker.is_some() {
            self.ui_picker(ctx);
        }
        if self.show_share {
            self.ui_share(ctx);
        }
        if self.remote_ctx.is_some() {
            self.ui_remote_ctx(ctx);
        }
        // Liability notice on top of everything, on first run.
        self.ui_disclaimer(ctx);

        // Alt key-overlay badges, drawn last so they sit above the toolbar/tabs.
        if self.accel_mode {
            self.draw_accel_overlay(ctx);
        }

        // Drag-over hint while the OS is dragging files onto the window.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            self.ui_drop_overlay(ctx);
            ctx.request_repaint();
        }

        // Internal file drag (between tabs/panes; out to Explorer on Windows).
        self.handle_file_drag(ctx);

        // Trackpad scrolling: egui spreads each scroll delta over several frames
        // (exponential smoothing) but does NOT request those frames itself, so a
        // reactive app only repaints on the discrete OS events → the glide
        // stalls and stutters. Keep painting at full rate during scrolling and
        // for a short tail afterwards, so the smoothing runs to a clean stop.
        if ctx.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO) {
            self.last_scroll_at = Some(std::time::Instant::now());
        }
        if let Some(t) = self.last_scroll_at {
            if t.elapsed() < std::time::Duration::from_millis(900) {
                ctx.request_repaint();
            } else {
                self.last_scroll_at = None;
            }
        }

    }
}
