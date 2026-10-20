//! Two pages of the vault properties that show things outside the vault
//! list (U29-7): „Sync“ (the vault jobs of the site) and „Laufwerk“ (the
//! drive of the site and the switch „Beim Öffnen einhängen“).
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_actions_ui;
use super::vault_feed::{poll, spawn, Poll};
use super::vault_models_sync::{broken_vault_job_lines, schedule_text};
use super::vault_sidebar_ui::Effect;
use super::vault_ui_settings;
use crate::mount::{MountSnapshot, MountSource};

/// How often the lists look again while their page is open.
const REFRESH: Duration = Duration::from_secs(3);

struct JobLine {
    id: String,
    name: String,
    schedule: String,
    enabled: bool,
}

/// The page „Sync“.
#[derive(Default)]
pub(super) struct SyncInfo {
    jobs: Vec<JobLine>,
    broken: Vec<String>,
    loaded_at: Option<Instant>,
    error: Option<String>,
}

impl SyncInfo {
    fn load(&mut self, site: &str) {
        self.loaded_at = Some(Instant::now());
        let report = match crate::syncjobs::load_report() {
            Ok(report) => report,
            Err(error) => {
                self.error = Some(format!("Sync-Jobs lesen: {error}"));
                return;
            }
        };
        self.error = None;
        self.jobs = report
            .jobs
            .iter()
            .filter(|job| job.is_vault() && (job.vault_site_a == site || job.vault_site_b == site))
            .map(|job| JobLine {
                id: job.id.clone(),
                name: job.name.clone(),
                schedule: schedule_text(job),
                enabled: job.enabled,
            })
            .collect();
        self.broken = broken_vault_job_lines(&report);
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, site: &str, effects: &mut Vec<Effect>) {
        if self.loaded_at.is_none_or(|at| at.elapsed() >= REFRESH) {
            self.load(site);
        }
        ui.ctx().request_repaint_after(REFRESH);
        if let Some(error) = &self.error {
            ui.colored_label(crate::app::theme::danger(ui), error);
        }
        if self.jobs.is_empty() && self.broken.is_empty() {
            ui.label("Kein Tresor-Sync-Job nutzt diesen Tresorort.");
        }
        for job in &self.jobs {
            ui.group(|ui| {
                ui.label(egui::RichText::new(&job.name).strong());
                ui.small(if job.enabled {
                    format!("aktiv · {}", job.schedule)
                } else {
                    format!("deaktiviert · {}", job.schedule)
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Jetzt ausführen").clicked() {
                        effects.push(Effect::RunJob(job.id.clone()));
                    }
                    if ui.button("Bearbeiten").clicked() {
                        effects.push(Effect::EditJob(job.id.clone()));
                    }
                    if ui.button("Vergleichen").clicked() {
                        effects.push(Effect::OpenCompare {
                            job: job.id.clone(),
                            name: job.name.clone(),
                        });
                    }
                });
            });
        }
        for text in &self.broken {
            ui.colored_label(crate::app::theme::warning(ui), text);
        }
        if ui.button("Synchronisieren…").clicked() {
            effects.push(Effect::NewSync(site.to_string()));
        }
    }
}

type MountAnswer = Result<Vec<MountSnapshot>, String>;

/// The page „Laufwerk“.
#[derive(Default)]
pub(super) struct DriveInfo {
    rx: Option<Receiver<MountAnswer>>,
    mounts: Vec<MountSnapshot>,
    error: Option<String>,
    loaded_at: Option<Instant>,
    eject_rx: Option<Receiver<Result<(), String>>>,
    /// The drive runtime is ready (looked at with the list of drives).
    runtime_ready: bool,
    /// „Beim Öffnen einhängen“, read once when the page opens.
    on_open: Option<bool>,
}

impl DriveInfo {
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        site: &str,
        label: &str,
        unlocked: bool,
        service_running: bool,
        effects: &mut Vec<Effect>,
    ) {
        let ctx = ui.ctx().clone();
        let mut reload = false;
        match poll(&mut self.rx) {
            Poll::Ready(Ok(mounts)) => {
                self.mounts = mounts;
                self.error = None;
            }
            Poll::Ready(Err(error)) => self.error = Some(error),
            Poll::Gone => self.error = Some("Die Abfrage endete ohne Antwort.".to_string()),
            Poll::Pending => {}
        }
        match poll(&mut self.eject_rx) {
            Poll::Ready(Ok(())) => reload = true,
            Poll::Ready(Err(error)) => self.error = Some(format!("Auswerfen: {error}")),
            Poll::Gone => self.error = Some("Das Auswerfen endete ohne Antwort.".to_string()),
            Poll::Pending => {}
        }
        // Asking for the drives starts the service; without one there is none.
        let due = self.loaded_at.is_none_or(|at| at.elapsed() >= REFRESH);
        if due || reload {
            self.loaded_at = Some(Instant::now());
            self.runtime_ready = crate::mount::drive_runtime_ready();
        }
        if (due || reload) && self.rx.is_none() && service_running {
            match spawn(&ctx, "vault-drive-list", || crate::daemon::list_mounts()) {
                Ok(rx) => self.rx = Some(rx),
                Err(error) => self.error = Some(error),
            }
        }
        ctx.request_repaint_after(REFRESH);
        let mine = self.mounts.iter().find(|snapshot| {
            matches!(&snapshot.config.source, MountSource::Vault { site_id } if site_id == site)
        });
        match mine {
            Some(snapshot) => {
                ui.label(crate::app::mount_ui_helpers::status_label(&snapshot.status));
                if let Some(path) = crate::app::mount_ui_open::mounted_location(&snapshot.status) {
                    ui.label(format!("Einhängepfad: {}", path.display()));
                }
                if matches!(
                    snapshot.recovery,
                    crate::mount::MountRecovery::RequiresUnlock { .. }
                ) {
                    ui.label(
                        "requires-unlock: Änderungen warten verschlüsselt auf das Entsperren.",
                    );
                }
            }
            None => {
                ui.label("Der Tresor ist nicht als Laufwerk eingehängt.");
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(crate::app::theme::danger(ui), error);
        }
        ui.horizontal_wrapped(|ui| {
            let mounted = mine.is_some_and(|snapshot| {
                crate::app::mount_ui_open::mounted_location(&snapshot.status).is_some()
            });
            if ui
                .add_enabled(
                    unlocked && !mounted && self.runtime_ready,
                    egui::Button::new("Als Laufwerk einhängen"),
                )
                .clicked()
            {
                effects.extend(vault_actions_ui::mount_drive(&ctx, site, label));
                self.loaded_at = None;
            }
            if let Some(snapshot) = mine {
                if ui
                    .add_enabled(self.eject_rx.is_none(), egui::Button::new("Auswerfen"))
                    .clicked()
                {
                    let id = snapshot.config.id.clone();
                    match spawn(&ctx, "vault-drive-eject", move || {
                        crate::daemon::stop_mount(id).map(|_| ())
                    }) {
                        Ok(rx) => self.eject_rx = Some(rx),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        });
        ui.separator();
        let mut on = *self
            .on_open
            .get_or_insert_with(vault_ui_settings::mount_on_open);
        if ui
            .checkbox(&mut on, "Beim Öffnen einhängen")
            .on_hover_text(
                "Beim Öffnen einer Tresordatei den Tresor als Laufwerk einhängen, \
                 statt eine Klartextkopie anzubieten. Gilt für alle Tresore.",
            )
            .changed()
        {
            match vault_ui_settings::set_mount_on_open(on) {
                Ok(()) => self.on_open = Some(on),
                Err(error) => self.error = Some(format!("Einstellung speichern: {error}")),
            }
        }
    }
}
