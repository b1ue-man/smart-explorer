use super::prelude::*;
use super::*;
use crate::app::theme;

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

    pub(in crate::app) fn ui_status(&mut self, ui: &mut egui::Ui) {
        let sel_bytes = self.selection_bytes();
        let progress = self.progress.clone();
        let transfers: Vec<(usize, TransferProgress, bool)> = self
            .transfers
            .active
            .iter()
            .enumerate()
            .filter(|(_, transfer)| !transfer.progress.done)
            .map(|(index, transfer)| (index, transfer.progress.clone(), transfer.canceling()))
            .collect();
        let queued_transfers = self.transfers.queued_len();
        let copy = self
            .copy_progress
            .as_ref()
            .filter(|progress| !progress.done)
            .cloned();
        let sync_progress = self.sync_progress.clone();
        let delete_progress = self.trash_progress.clone();
        let delete_canceling = self
            .trash_cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Acquire));
        let notice = self.notice.clone();
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| self.ui_read_access(ui));
            ui.horizontal_wrapped(|ui| {
                if self.scan_running {
                    if self.scan_handle.is_some() {
                        ui.label("⟳ Scan läuft…");
                    } else {
                        ui.colored_label(
                            theme::warning(ui),
                            "⏹ Scan wird abgebrochen…",
                        );
                    }
                } else if self.scan_was_canceled {
                    ui.colored_label(
                        theme::warning(ui),
                        "⚠ Scan abgebrochen · Teilergebnis",
                    );
                } else if progress.errors > 0 {
                    ui.colored_label(
                        theme::warning(ui),
                        "⚠ Scan teilweise abgeschlossen",
                    );
                } else if !self.entries.is_empty() {
                    ui.label("Bereit");
                }
                let p = &progress;
                if !self.root_path.is_empty() {
                    let text = if self.scan_running {
                        format!("{} gescannt · {}", p.scanned, format_bytes(p.bytes))
                    } else {
                        format!("{} Einträge · {}", self.tree.rows.len(), format_bytes(p.bytes))
                    };
                    ui.colored_label(theme::muted(ui), text).on_hover_text(format!(
                        "{} gescannt · {:.1} Sekunden · {} Fehler", p.scanned, p.elapsed_ms as f64 / 1000.0, p.errors));
                }
                if !p.current_path.is_empty() && self.scan_running {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&p.current_path).monospace().small())
                            .truncate(),
                    )
                    .on_hover_text(&p.current_path);
                }
            });
            let has_details = !transfers.is_empty() || queued_transfers > 0 || copy.is_some() || self.sync_running
                || self.bisync_running || delete_progress.is_some()
                || notice.as_ref().is_some_and(|(_, time)| time.elapsed().as_secs() < 6)
                || self.error_msg.is_some() || progress.errors > 0
                || !self.failed_paths.is_empty() || !self.app_errors.is_empty()
                || !self.selection.is_empty();
            if !has_details { return; }
            ui.horizontal_wrapped(|ui| {
                for (index, p, canceling) in &transfers {
                    ui_transfer_chip(ui, p);
                    if *canceling {
                        ui.colored_label(
                            theme::warning(ui),
                            "Übertragung wird abgebrochen…",
                        );
                    } else if ui
                        .add(egui::Button::new("Abbrechen").small())
                        .on_hover_text("Diese Übertragung nach dem laufenden Backend-Aufruf sicher abbrechen")
                        .clicked()
                    {
                        self.transfers.cancel(*index);
                    }
                }
                if queued_transfers > 0 {
                    ui.colored_label(
                        theme::muted(ui),
                        format!("{queued_transfers} Übertragung(en) wartend"),
                    )
                    .on_hover_text(format!(
                        "Bis zu {} Übertragungen laufen gleichzeitig und teilen sich die Bandbreite; weitere starten automatisch.",
                        super::transfer_jobs::MAX_ACTIVE_TRANSFERS
                    ));
                }
                if (transfers.len() > 1 || queued_transfers > 0)
                    && ui
                        .add(egui::Button::new("Alle Übertragungen abbrechen").small())
                        .clicked()
                {
                    self.transfers.cancel_all();
                }
                if let Some(p) = &copy {
                    ui_copy_chip(ui, p);
                    if ui
                        .add(egui::Button::new("Kopie abbrechen").small())
                        .on_hover_text("Laufenden Kopier- oder Verschiebevorgang abbrechen")
                        .clicked()
                    {
                        self.cancel_copy_job();
                    }
                }
                if self.sync_running {
                    ui_sync_chip(ui, sync_progress.as_ref());
                    if ui
                        .add(egui::Button::new("Sync abbrechen").small())
                        .clicked()
                    {
                        if let Some(cancel) = &self.sync_cancel {
                            cancel.store(true, std::sync::atomic::Ordering::Release);
                        }
                    }
                }
                if self.bisync_running {
                    ui.colored_label(theme::accent(ui), "2-Wege-Sync läuft…");
                    if ui
                        .add(egui::Button::new("2-Wege-Sync abbrechen").small())
                        .clicked()
                    {
                        if let Some(cancel) = &self.bisync_cancel {
                            cancel.store(true, std::sync::atomic::Ordering::Release);
                        }
                    }
                }
                if let Some(progress) = &delete_progress {
                    if crate::app::delete_status::ui_delete_progress(ui, progress, delete_canceling)
                    {
                        self.cancel_delete_job();
                    }
                }
                if let Some((ref msg, ts)) = notice {
                    if ts.elapsed().as_secs() < 6 {
                        ui.colored_label(notice_color(ui, msg), msg.as_str());
                    }
                }
                if let Some(ref e) = self.error_msg {
                    ui.colored_label(theme::danger(ui), format!("⚠ {}", e));
                }
                let scan_errors = progress.errors.max(self.failed_paths.len() as u64) as usize;
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
                                RichText::new(label).color(theme::danger(ui)),
                            )
                            .small(),
                        )
                        .on_hover_text("Fehler-Protokoll anzeigen und kopieren")
                        .clicked()
                    {
                        self.show_errors_dialog = true;
                    }
                }
                if !self.selection.is_empty() {
                    ui.colored_label(
                        theme::muted(ui),
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
    let current = short_current(&p.current);
    let text = if current.is_empty() {
        format!("{title}: {detail}")
    } else {
        format!("{title}: {detail} · {current}")
    };
    ui_progress_chip(ui, &text, Some(p.fraction()));
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
        let current = short_current(&p.current);
        let text = if current.is_empty() {
            format!("Sync: {detail}")
        } else {
            format!("Sync: {detail} · {current}")
        };
        ui_progress_chip(ui, &text, None);
    } else {
        ui_progress_chip(ui, "Sync laeuft...", None);
    }
}

fn ui_progress_chip(ui: &mut egui::Ui, text: &str, fraction: Option<f32>) {
    let width = ui.available_width().clamp(180.0, 360.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let bar = match fraction {
                Some(fraction) => egui::ProgressBar::new(fraction.clamp(0.0, 1.0)),
                None => egui::ProgressBar::new(0.35).animate(true),
            };
            ui.add(bar.desired_width(76.0).desired_height(6.0));
            ui.add(
                egui::Label::new(RichText::new(text).small().color(theme::muted(ui))).truncate(),
            )
            .on_hover_text(text);
        },
    );
}

fn short_current(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn notice_color(ui: &egui::Ui, message: &str) -> Color32 {
    let lower = message.to_lowercase();
    if message.starts_with('⚠')
        || lower.contains("konflikt")
        || lower.contains("teilweise")
        || lower.contains("abgebrochen")
    {
        theme::warning(ui)
    } else if lower.contains("fehler") || lower.contains("fehlgeschlagen") {
        theme::danger(ui)
    } else {
        theme::success(ui)
    }
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
