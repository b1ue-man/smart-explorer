use crate::app::theme;
use super::bisync_conflicts::{ConflictBulkRun, ConflictSide};
use super::prelude::*;
use super::*;

const CONFLICT_ROW_HEIGHT: f32 = 30.0;

impl App {
    pub(in crate::app) fn ui_bisync_conflicts(&mut self, ctx: &egui::Context) {
        if !self.show_bisync_conflicts {
            return;
        }
        if self.bisync_conflicts.is_empty() {
            self.finish_bisync_conflicts();
            return;
        }

        let mut keep: Option<(usize, ConflictSide, Option<String>)> = None;
        let mut skip = None;
        let mut merge_req = None;
        let mut close = false;
        let mut stop_or_cancel = false;
        let mut start_bulk = None;
        let worker_active = self.conflict_resolution.is_some();
        let merge_active =
            self.merge.is_some() || self.merge_load_rx.is_some() || self.merge_apply_rx.is_some();
        let bulk_active = self.conflict_bulk.is_some();
        let choices_disabled = worker_active || merge_active || bulk_active;
        let conflict_count = self.bisync_conflicts.len();

        egui::Window::new(format!("⚠ Sync-Konflikte ({conflict_count})"))
            .collapsible(false)
            .resizable(true)
            .default_size([700.0, 460.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Wähle die Version für beide Seiten. Ersetzte Versionen werden gesichert; doppelte Dateien anschließend entfernt.");
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !choices_disabled,
                            egui::Button::new(format!(
                                "Alle {conflict_count}: A verwenden (A → B)"
                            )),
                        )
                        .clicked()
                    {
                        start_bulk = Some(ConflictSide::A);
                    }
                    if ui
                        .add_enabled(
                            !choices_disabled,
                            egui::Button::new(format!(
                                "Alle {conflict_count}: B verwenden (B → A)"
                            )),
                        )
                        .clicked()
                    {
                        start_bulk = Some(ConflictSide::B);
                    }
                });
                self.ui_conflict_progress(ui, &mut stop_or_cancel);
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(
                        ui,
                        CONFLICT_ROW_HEIGHT,
                        self.bisync_conflicts.len(),
                        |ui, visible_rows| {
                            for index in visible_rows {
                                let conflict = &self.bisync_conflicts[index];
                                ui.horizontal(|ui| {
                                    for side in [ConflictSide::A, ConflictSide::B] {
                                        if let Some(id) = variant_choice(ui, conflict, side, !choices_disabled) {
                                            keep = Some((index, side, id));
                                        }
                                    }
                                    if ui
                                        .add_enabled(
                                            !choices_disabled && conflict.duplicates.is_none(),
                                            egui::Button::new("⇄ Zeilen").small(),
                                        )
                                        .on_hover_text("Zeilenweise zusammenführen")
                                        .clicked()
                                    {
                                        merge_req = Some(index);
                                    }
                                    if ui
                                        .add_enabled(
                                            !choices_disabled,
                                            egui::Button::new("⏭").small(),
                                        )
                                        .on_hover_text("Vorerst überspringen")
                                        .clicked()
                                    {
                                        skip = Some(index);
                                    }
                                    ui.add(egui::Label::new(&conflict.rel).truncate())
                                        .on_hover_text(conflict.rel.as_str());
                                });
                            }
                        },
                    );
                ui.add_space(6.0);
                if ui
                    .add_enabled(
                        !worker_active && !bulk_active && !merge_active,
                        egui::Button::new("Schließen (Rest später)"),
                    )
                    .clicked()
                {
                    close = true;
                }
            });

        if stop_or_cancel {
            self.cancel_conflict_resolution();
        }
        if let Some(side) = start_bulk {
            let description = match side {
                ConflictSide::A => "A nach B",
                ConflictSide::B => "B nach A",
            };
            if confirm_yes_no(
                "Alle Konflikte auflösen",
                &format!(
                    "{conflict_count} Konflikte automatisch mit {description} auflösen? Die ersetzten Versionen werden reversibel gesichert."
                ),
            ) {
                self.conflict_bulk = Some(ConflictBulkRun::new(side, conflict_count));
            }
        }
        if close {
            self.finish_bisync_conflicts();
        } else if let Some((index, side, id)) = keep {
            self.start_conflict_variant_resolution(index, side, false, id);
        } else if let Some(index) = skip {
            if index < self.bisync_conflicts.len() {
                self.bisync_conflicts.swap_remove(index);
            }
            if self.bisync_conflicts.is_empty() {
                self.finish_bisync_conflicts();
            }
        } else if let Some(index) = merge_req {
            if let Some(conflict) = self.bisync_conflicts.get(index) {
                self.start_merge(conflict.rel.clone());
            }
        }

        if self.conflict_resolution.is_none() {
            if let Some(side) = self.conflict_bulk.as_ref().map(|bulk| bulk.side) {
                if !self.start_conflict_resolution(0, side, true) {
                    self.conflict_bulk = None;
                }
            }
        }
        if self.conflict_resolution.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn ui_conflict_progress(&self, ui: &mut egui::Ui, stop_or_cancel: &mut bool) {
        if let Some(task) = &self.conflict_resolution {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                let phase = if task.cancel_requested() {
                    "Abbruch angefordert; sichere Grenze wird abgewartet"
                } else {
                    resolve_phase_label(task.phase)
                };
                ui.label(format!(
                    "{}: {} · {}",
                    task.side.direction(),
                    task.rel,
                    phase
                ));
                let label = if self.conflict_bulk.is_some() {
                    "Stapel stoppen"
                } else {
                    "Auflösung abbrechen"
                };
                if ui.button(label).clicked() {
                    *stop_or_cancel = true;
                }
            });
        }
        if let Some(bulk) = &self.conflict_bulk {
            let fraction = if bulk.total == 0 {
                0.0
            } else {
                bulk.completed as f32 / bulk.total as f32
            };
            ui.add(
                egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .show_percentage()
                    .text(format!(
                        "Stapel: {} von {} abgeschlossen · {}",
                        bulk.completed,
                        bulk.total,
                        bulk.side.direction()
                    )),
            );
        }
    }


}

fn resolve_phase_label(phase: crate::bisync::ResolvePhase) -> &'static str {
    match phase {
        crate::bisync::ResolvePhase::Preparing => "prüft beide Seiten",
        crate::bisync::ResolvePhase::BackingUp => "sichert die ersetzte Version",
        crate::bisync::ResolvePhase::Copying => "überträgt die gewählte Version",
        crate::bisync::ResolvePhase::Deleting => "übernimmt die Löschung",
        crate::bisync::ResolvePhase::ReadingSignatures => "prüft das Ergebnis",
    }
}

fn variant_choice(
    ui: &mut egui::Ui, conflict: &crate::bisync::Conflict,
    side: ConflictSide, enabled: bool,
) -> Option<Option<String>> {
    let letter = if side.keep_a() { "A" } else { "B" };
    let mut selected = None;
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(group) = conflict.duplicates.as_ref().filter(|d| d.needs_variant_choice(side.keep_a())) {
            ui.menu_button(format!("{letter}: Version wählen…"), |ui| {
                ui.label("Mehrere Dateien mit demselben Namen. Diese Version bleibt auf beiden Seiten:");
                for (index, variant) in group.variants(side.keep_a()).iter().enumerate() {
                    let label = format!("Version {} · {} B · {} · Inhalt {}", index + 1,
                        variant.content_size, fmt_ms(variant.signature.mtime_ms),
                        variant.content_md5.chars().take(8).collect::<String>());
                    if ui.button(label).on_hover_text(format!("Datei-ID: {}\nPrüfsumme: {}",
                        variant.id.as_deref().unwrap_or("Pfad"), variant.content_md5)).clicked() {
                        selected = Some(variant.id.clone());
                        ui.close_menu();
                    }
                }
            });
        } else {
            let signature = if side.keep_a() { conflict.a } else { conflict.b };
            let detail = signature.map(|s| format!("{} B, {}", s.size, fmt_ms(s.mtime_ms)))
                .unwrap_or_else(|| "gelöscht".into());
            if ui.small_button(format!("{letter} verwenden ({})", side.direction()))
                .on_hover_text(detail).clicked() { selected = Some(None); }
        }
    });
    selected
}
