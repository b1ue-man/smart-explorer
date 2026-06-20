use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn selection_bytes(&mut self) -> u64 {
        if self.sel_size_cache.0 == self.selection.len()
            && self.sel_size_cache.1 == self.entries.len()
        {
            return self.sel_size_cache.2;
        }
        let b: u64 = self
            .entries
            .iter()
            .filter(|e| !e.is_dir && self.selection.contains(&e.key()))
            .map(|e| e.size)
            .sum();
        self.sel_size_cache = (self.selection.len(), self.entries.len(), b);
        b
    }

    pub(in crate::app) fn push_app_error(
        &mut self,
        context: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let detail = detail.into();
        if detail.trim().is_empty() {
            return;
        }
        if self.last_logged_error.as_deref() == Some(detail.as_str()) {
            return;
        }
        self.last_logged_error = Some(detail.clone());
        self.app_errors.push(AppErrorEntry {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            context: context.into(),
            detail,
        });
        if self.app_errors.len() > APP_ERROR_LOG_LIMIT {
            let remove = self.app_errors.len() - APP_ERROR_LOG_LIMIT;
            self.app_errors.drain(0..remove);
        }
    }

    pub(in crate::app) fn capture_current_error(&mut self) {
        if let Some(detail) = self.error_msg.clone() {
            self.push_app_error("Fehler", detail);
        } else {
            self.last_logged_error = None;
        }
    }

    pub(in crate::app) fn error_log_text(&self) -> String {
        let mut lines = Vec::new();
        if !self.app_errors.is_empty() {
            lines.push("App-Fehler:".to_string());
            for e in &self.app_errors {
                lines.push(format!("[{}] {}: {}", e.ts, e.context, e.detail));
            }
        }
        if let Some(current) = &self.error_msg {
            if !self.app_errors.iter().any(|e| e.detail == *current) {
                if lines.is_empty() {
                    lines.push("App-Fehler:".to_string());
                }
                lines.push(format!("[aktuell] Fehler: {}", current));
            }
        }
        if !self.failed_paths.is_empty() || self.progress.errors > 0 {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(format!(
                "Scan-Fehler: {} gesamt, {} Pfade im Protokoll",
                self.progress.errors,
                self.failed_paths.len()
            ));
            for (path, msg) in &self.failed_paths {
                lines.push(format!("{}\t{}", path, msg));
            }
        }
        lines.join("\r\n")
    }

    pub(in crate::app) fn ui_status(&mut self, ui: &mut egui::Ui) {
        let sel_bytes = self.selection_bytes();
        ui.horizontal(|ui| {
            if self.scan_running {
                ui.label("⟳ Scan läuft…");
            } else if !self.entries.is_empty() {
                ui.label("✓ Bereit");
            }
            let p = &self.progress;
            let rate = if p.elapsed_ms > 0 {
                (p.scanned as f64 / p.elapsed_ms as f64) * 1000.0
            } else {
                0.0
            };
            let rate_s = if rate >= 1000.0 {
                format!("{:.1}k/s", rate / 1000.0)
            } else {
                format!("{:.0}/s", rate)
            };
            ui.colored_label(
                Color32::from_gray(140),
                format!(
                    "{} gescannt · {} · {:.1}s · {}{}",
                    p.scanned,
                    format_bytes(p.bytes),
                    p.elapsed_ms as f64 / 1000.0,
                    rate_s,
                    if p.errors > 0 {
                        format!(" · {} Fehler", p.errors)
                    } else {
                        String::new()
                    },
                ),
            );
            if !p.current_path.is_empty() && self.scan_running {
                ui.colored_label(
                    Color32::from_gray(110),
                    egui::RichText::new(&p.current_path).monospace().small(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(
                    Color32::from_gray(140),
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                );
                if let Some(p) = self.transfer_progress.as_ref().filter(|p| !p.done) {
                    ui_transfer_chip(ui, p);
                }
                if let Some(p) = self.copy_progress.as_ref().filter(|p| !p.done) {
                    ui_copy_chip(ui, p);
                }
                if self.sync_running {
                    ui_sync_chip(ui, self.sync_progress.as_ref());
                }
                if self.bisync_running {
                    ui.colored_label(Color32::from_rgb(160, 190, 230), "2-Wege-Sync laeuft...");
                }
                if let Some((ref msg, ts)) = self.notice {
                    if ts.elapsed().as_secs() < 6 {
                        ui.colored_label(Color32::from_rgb(120, 200, 130), msg.clone());
                    }
                }
                if let Some(ref e) = self.error_msg {
                    ui.colored_label(Color32::from_rgb(220, 100, 80), format!("⚠ {}", e));
                }
                let scan_errors = p.errors.max(self.failed_paths.len() as u64) as usize;
                let app_errors = if self.app_errors.is_empty() && self.error_msg.is_some() {
                    1
                } else {
                    self.app_errors.len()
                };
                let total_errors = scan_errors + app_errors;
                if total_errors > 0 {
                    let label = format!("⚠ {} Fehler", total_errors);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(label).color(Color32::from_rgb(220, 100, 80)),
                            )
                            .small(),
                        )
                        .on_hover_text("Fehler-Protokoll anzeigen und kopieren")
                        .clicked()
                    {
                        self.show_errors_dialog = true;
                    }
                }
                if self.selection.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "Auswahl: 0");
                } else {
                    ui.colored_label(
                        Color32::from_gray(160),
                        format!(
                            "Auswahl: {} ({})",
                            self.selection.len(),
                            format_bytes(sel_bytes)
                        ),
                    );
                }
            });
        });
    }

    pub(in crate::app) fn ui_errors_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut clear_app_log = false;
        let mut log_text = self.error_log_text();
        let scan_errors = self.progress.errors.max(self.failed_paths.len() as u64) as usize;
        let app_errors = if self.app_errors.is_empty() && self.error_msg.is_some() {
            1
        } else {
            self.app_errors.len()
        };
        egui::Window::new(format!("Fehler-Protokoll ({})", scan_errors + app_errors))
            .resizable(true)
            .default_size([700.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Fehler aus der App und nicht lesbare Scan-Pfade. Der Text ist markierbar und kopierbar.");
                ui.add_space(6.0);
                if log_text.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "Keine Fehler protokolliert.");
                } else {
                    ui.add(
                        egui::TextEdit::multiline(&mut log_text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(18),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Alles kopieren").clicked() {
                        ctx.copy_text(log_text.clone());
                    }
                    if !self.app_errors.is_empty() && ui.button("App-Protokoll leeren").clicked()
                    {
                        clear_app_log = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Schließen").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if clear_app_log {
            self.app_errors.clear();
            self.last_logged_error = None;
        }
        if close {
            self.show_errors_dialog = false;
        }
    }
}

fn ui_transfer_chip(ui: &mut egui::Ui, p: &TransferProgress) {
    let title = if p.label.trim().is_empty() {
        p.kind.label().to_string()
    } else if p.label == p.kind.label() {
        p.label.clone()
    } else {
        format!("{}: {}", p.kind.label(), p.label)
    };
    let detail = transfer_detail(
        p.bytes_done,
        p.bytes_total,
        p.files_done,
        p.files_total,
        p.elapsed_ms,
        p.errors,
    );
    ui_progress_chip(ui, &format!("{title}: {detail}"), Some(p.fraction()));
}

fn ui_copy_chip(ui: &mut egui::Ui, p: &CopyProgress) {
    let detail = transfer_detail(
        p.bytes_done,
        p.bytes_total,
        p.files_done,
        p.files_total,
        p.elapsed_ms,
        p.errors,
    );
    ui_progress_chip(ui, &format!("Kopie: {detail}"), Some(copy_fraction(p)));
}

fn ui_sync_chip(ui: &mut egui::Ui, progress: Option<&crate::sync::SyncProgress>) {
    if let Some(p) = progress {
        let rate = rate_text(p.stats.bytes, p.elapsed_ms);
        let detail = format!(
            "{} kopiert, {} geloescht, {} | {}",
            p.stats.copied,
            p.stats.deleted,
            format_bytes(p.stats.bytes),
            rate
        );
        ui_progress_chip(ui, &format!("Sync: {detail}"), None);
    } else {
        ui_progress_chip(ui, "Sync laeuft...", None);
    }
}

fn ui_progress_chip(ui: &mut egui::Ui, text: &str, fraction: Option<f32>) {
    ui.allocate_ui_with_layout(
        egui::vec2(280.0, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let bar = match fraction {
                Some(fraction) => egui::ProgressBar::new(fraction.clamp(0.0, 1.0)),
                None => egui::ProgressBar::new(0.35).animate(true),
            };
            ui.add(bar.desired_width(76.0).desired_height(6.0));
            ui.colored_label(Color32::from_gray(160), RichText::new(text).small());
        },
    );
}

fn transfer_detail(
    bytes_done: u64,
    bytes_total: u64,
    files_done: u64,
    files_total: u64,
    elapsed_ms: u64,
    errors: u64,
) -> String {
    let bytes = if bytes_total > 0 {
        format!("{}/{}", format_bytes(bytes_done), format_bytes(bytes_total))
    } else {
        format_bytes(bytes_done)
    };
    let files = if files_total > 0 {
        format!("{} von {}", files_done, files_total)
    } else {
        format!("{} Dateien", files_done)
    };
    let err = if errors > 0 {
        format!(" | {} Fehler", errors)
    } else {
        String::new()
    };
    format!(
        "{} | {} | {}{}",
        bytes,
        rate_text(bytes_done, elapsed_ms),
        files,
        err
    )
}

fn rate_text(bytes_done: u64, elapsed_ms: u64) -> String {
    if elapsed_ms == 0 {
        return "0 B/s".to_string();
    }
    let bps = (bytes_done as f64 / elapsed_ms as f64 * 1000.0).max(0.0);
    format!("{}/s", format_bytes(bps as u64))
}

fn copy_fraction(p: &CopyProgress) -> f32 {
    if p.bytes_total > 0 {
        (p.bytes_done as f32 / p.bytes_total as f32).clamp(0.0, 1.0)
    } else if p.files_total > 0 {
        (p.files_done as f32 / p.files_total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    }
}
