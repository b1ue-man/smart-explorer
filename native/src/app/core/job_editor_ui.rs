use super::job_editor_pages::{render_job_editor_page, JobEditorPage};
use super::prelude::*;
use super::*;
use crate::app::theme;

impl App {
    /// Add/edit dialog for a single sync setup. Every setting of the shared
    /// `JobEditor` sits on one of the pages in `job_editor_pages.rs`
    /// (Allgemein, Zeitplan, Vergleich & Versionen, Filter, Übertragung &
    /// Befehle); validation and saving always cover all pages.
    pub(in crate::app) fn ui_job_editor(&mut self, ctx: &egui::Context) {
        let mut ed = match self.job_editor.take() {
            Some(e) => e,
            None => return,
        };
        let mut open = true;
        let existing_job = ed
            .id
            .as_ref()
            .and_then(|id| self.sync_jobs.iter().find(|job| &job.id == id))
            .cloned();
        let mut job_to_save = None;
        let busy = ed.id.as_deref().is_some_and(|id| self.desktop_job_busy(id));
        let mut cancel = false;
        // Set when a "Durchsuchen" button is clicked → open the in-app picker
        // after `ed` is restored to self.job_editor (so the picker can write
        // back into it). Carries the field + its current value as a start point.
        let mut pick: Option<(PickerPurpose, String)> = None;
        let title = if ed.id.is_some() {
            "✎ Sync-Setup bearbeiten"
        } else {
            "＋ Neues Sync-Setup"
        };
        // The page stays selected while the editor of this setup is open.
        let page_id = egui::Id::new(("job_editor_page", ed.id.clone()));
        let mut page = ctx
            .data_mut(|data| data.get_temp::<JobEditorPage>(page_id))
            .unwrap_or_default();
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([600.0, 520.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for candidate in JobEditorPage::ALL {
                        ui.selectable_value(&mut page, candidate, candidate.label());
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("job_editor_scroll", page as u8))
                    .max_height((ctx.screen_rect().height() - 200.0).max(180.0))
                    .show(ui, |ui| render_job_editor_page(ui, page, &mut ed, &mut pick));
                ui.separator();
                let validation = ed.build_sync_job(existing_job.as_ref());
                if let Err(error) = &validation {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::danger(ui), error);
                        // Every page is validated; this jumps to the field.
                        let target = JobEditorPage::for_error(error);
                        if target != page
                            && ui
                                .small_button(format!("→ {}", target.label()))
                                .clicked()
                        {
                            page = target;
                        }
                    });
                }
                if busy {
                    ui.colored_label(
                        theme::warning(ui),
                        "Dieses Setup ist noch in einem Lauf oder einer Konfliktauflösung geöffnet. Vor dem Speichern bitte abschließen.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(validation.is_ok() && !busy, egui::Button::new("✔ Speichern"))
                        .clicked()
                    {
                        job_to_save = validation.ok();
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                });
            });
        ctx.data_mut(|data| data.insert_temp(page_id, page));
        if cancel || !open {
            // Dropped (taken at top) — leaving job_editor as None closes it.
            return;
        }
        if let Some(job) = job_to_save {
            match crate::syncjobs::upsert(&job) {
                Ok(_) => match crate::syncjobs::load() {
                    Ok(jobs) => {
                        self.sync_jobs = jobs;
                        self.notice = Some((
                            format!("✓ Setup „{}“ gespeichert", job.name),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(error) => {
                        self.error_msg = Some(format!(
                            "Setup gespeichert, aber nicht neu geladen: {error}"
                        ));
                    }
                },
                Err(e) => {
                    self.error_msg = Some(format!("Setup speichern: {}", e));
                    self.job_editor = Some(ed);
                }
            }
            return;
        }
        // Still open, nothing pressed — keep the editor for the next frame.
        self.job_editor = Some(ed);
        // Now that job_editor is restored, the picker can write back into it.
        if let Some((purpose, initial)) = pick {
            self.open_picker(purpose, &initial);
        }
    }
}
