//! Einstellungen → Integration → „Verbundene Geräte als Ordner“ (DRV-M6,
//! V14: the section before „se Desktop“): switch, where it appears, state,
//! „Im Dateimanager öffnen“, changes that wait for a device, and what is
//! missing when the runtime is not ready.
//!
//! The state is read from the worker every 2 s on a thread of its own, but
//! only while this page is drawn; the switch runs on a thread too, so the
//! page never waits for the worker.
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::mount_ui_open::{open_button_label, open_in_file_manager_path};
use crate::app::prelude::*;
use crate::app::{theme, App};
use crate::daemon::DevicesStatus;
use crate::mount::MountStatus;

/// How old the shown state may get while the page is open.
const REFRESH: Duration = Duration::from_secs(2);
const EXPLANATION: &str = "Jedes gekoppelte Gerät erscheint als Ordner. Offline-Geräte bleiben \
sichtbar und melden ‚nicht erreichbar‘. Freigaben und Verbindungen des Geräts liegen darin.";

#[derive(Default)]
struct Panel {
    status: Option<DevicesStatus>,
    /// What is missing, when the runtime is not ready.
    runtime: Option<String>,
    error: Option<String>,
    loading: bool,
    loaded_at: Option<Instant>,
    busy: bool,
    /// Windows: `""` is automatic, else the letter; Linux: the folder.
    place: String,
}

type Shared = Arc<Mutex<Panel>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Panel> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads the state and the runtime on a thread, at most every `REFRESH`.
fn refresh(shared: &Shared, ctx: &egui::Context) {
    {
        let mut panel = lock(shared);
        let fresh = panel.loaded_at.is_some_and(|at| at.elapsed() < REFRESH);
        if panel.loading || panel.busy || fresh {
            return;
        }
        panel.loading = true;
    }
    let worker = shared.clone();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("devices-settings".into())
        .spawn(move || {
            let status = crate::daemon::devices_status();
            let runtime = runtime_problem();
            let mut panel = lock(&worker);
            match status {
                Ok(status) => {
                    panel.status = Some(status);
                    panel.error = None;
                }
                Err(error) => panel.error = Some(error),
            }
            panel.runtime = runtime;
            panel.loading = false;
            panel.loaded_at = Some(Instant::now());
            drop(panel);
            ctx.request_repaint();
        });
    if let Err(error) = spawned {
        let mut panel = lock(shared); panel.loading = false;
        panel.error = Some(format!("Laufwerksstatus konnte nicht geladen werden: {error}"));
    }
}

/// What the runtime lacks, in the words of the page; `None` when it is ready.
fn runtime_problem() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        match crate::mount::fuse_install_outcome() {
            crate::mount::FuseInstallOutcome::AlreadyReady => None,
            crate::mount::FuseInstallOutcome::ManualRequired { command, reason } => Some(format!(
                "Die Laufzeit fehlt: {}.",
                if command.is_empty() { reason } else { command }
            )),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        (!crate::mount::drive_runtime_ready())
            .then(|| "Dokany ist nicht installiert. Unter Laufwerke einrichten.".to_string())
    }
}

/// Switches the tree on or off on a thread.
fn switch(shared: &Shared, ctx: &egui::Context, on: bool, place: String) {
    lock(shared).busy = true;
    let worker = shared.clone();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("devices-switch".into())
        .spawn(move || {
            let place = Some(place.trim().to_string()).filter(|place| !place.is_empty());
            let result = if on {
                let (letter, folder) = if cfg!(windows) {
                    (place, None)
                } else {
                    (None, place)
                };
                crate::daemon::devices_enable(letter, folder)
            } else {
                crate::daemon::devices_disable()
            };
            let mut panel = lock(&worker);
            match result {
                Ok(status) => {
                    panel.status = Some(status);
                    panel.error = None;
                }
                Err(error) => panel.error = Some(error),
            }
            panel.busy = false;
            panel.loaded_at = Some(Instant::now());
            drop(panel);
            ctx.request_repaint();
        });
    if let Err(error) = spawned {
        let mut panel = lock(shared); panel.busy = false;
        panel.error = Some(format!("Laufwerksänderung konnte nicht gestartet werden: {error}"));
    }
}

/// The line „Zustand: …“. `reason` is why the drive failed, as the list of
/// drives names it.
fn state_text(status: &DevicesStatus, reason: Option<&str>) -> String {
    match status.state.as_str() {
        "off" => "Aus".into(),
        "mounting" => "Wird eingebunden …".into(),
        "mounted" => match (&status.letter, &status.root_path) {
            (Some(letter), _) => format!("Eingebunden als {letter}"),
            (None, Some(path)) => format!("Eingebunden in {path}"),
            (None, None) => "Eingebunden".into(),
        },
        "unmounting" => "Wird ausgehängt …".into(),
        "waiting" => "Wartet auf Smart Explorer Share".into(),
        "failed" => format!("Fehlgeschlagen: {}", reason.unwrap_or("unbekannter Grund")),
        "stopped" => "Angehalten".into(),
        _ => "Zustand nicht bekannt".into(),
    }
}

fn pending_text(pending: u32) -> String {
    format!("Nicht gespeicherte Änderungen für {pending} Gerät(e) warten auf eine Verbindung.")
}

impl App {
    /// The section; draws and polls only while Integration is open.
    pub(in crate::app) fn ui_settings_devices(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Verbundene Geräte als Ordner");
        let shared: Shared = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<Shared>(egui::Id::new("se_devices_settings"))
                .clone()
        });
        refresh(&shared, ui.ctx());
        ui.ctx().request_repaint_after(REFRESH);
        let (status, runtime, error, busy, mut place) = {
            let panel = lock(&shared);
            (
                panel.status.clone(),
                panel.runtime.clone(),
                panel.error.clone(),
                panel.busy,
                panel.place.clone(),
            )
        };
        let Some(status) = status else {
            match error {
                Some(error) => {
                    ui.label(RichText::new(error).color(theme::danger(ui)));
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Zustand wird gelesen …");
                    });
                }
            }
            return;
        };
        let mut enabled = status.enabled;
        let toggled = ui
            .add_enabled(
                !busy,
                egui::Checkbox::new(&mut enabled, "Verbundene Geräte als Ordner einbinden"),
            )
            .changed();
        let place_changed = ui_place(ui, &mut place, status.enabled, busy);
        if place_changed {
            lock(&shared).place = place.clone();
        }
        let reason = self
            .mount_ui
            .mounts
            .iter()
            .find(|mount| mount.config.source.is_device_tree())
            .and_then(|mount| match &mount.status {
                MountStatus::Failed { detail } | MountStatus::RuntimeUnavailable { detail } => {
                    Some(detail.clone())
                }
                _ => None,
            });
        ui.label(format!(
            "Zustand: {}",
            state_text(&status, reason.as_deref())
        ));
        if status.state == "mounted" {
            if let Some(root) = status.root_path.as_deref() {
                if ui
                    .add_enabled(!busy, egui::Button::new(open_button_label()))
                    .clicked()
                {
                    open_in_file_manager_path(self, root);
                }
            }
        }
        if status.pending > 0 {
            ui.label(RichText::new(pending_text(status.pending)).color(theme::warning(ui)));
        }
        ui.label(RichText::new(EXPLANATION).color(theme::muted(ui)));
        if let Some(problem) = runtime {
            ui.label(RichText::new(problem).color(theme::warning(ui)));
        }
        crate::app::settings_form::status(ui, busy, None, error.as_deref());
        if toggled {
            switch(&shared, ui.ctx(), enabled, place);
        }
    }
}

/// The choice of the letter (Windows) or the folder (Linux); fixed while
/// the tree is on. `true` when the text changed.
fn ui_place(ui: &mut egui::Ui, place: &mut String, enabled: bool, busy: bool) -> bool {
    let mut changed = false;
    ui.add_enabled_ui(!enabled && !busy, |ui| {
        let label = if cfg!(windows) { "Laufwerksbuchstabe" } else { "Einhängeort" };
        let mut form = crate::app::settings_form::Form::new(ui, "devices_place_form", &[label]);
        form.row_labelled(ui, label, |ui, field_label| {
            if cfg!(windows) {
                let shown = if place.is_empty() {
                    "Automatisch".to_string()
                } else {
                    place.clone()
                };
                egui::ComboBox::from_id_salt("se_devices_letter")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(place, String::new(), "Automatisch")
                            .changed();
                        for letter in 'D'..='Z' {
                            changed |= ui
                                .selectable_value(place, letter.to_string(), letter.to_string())
                                .changed();
                        }
                    }).response.labelled_by(field_label);
            } else {
                changed = ui
                    .add(
                        egui::TextEdit::singleline(place)
                            .hint_text("Standard (~/SmartExplorer/Geräte)")
                            .desired_width(280.0),
                    ).labelled_by(field_label)
                    .changed();
            }
        });
    })
    .response
    .on_disabled_hover_text("Zum Ändern erst ausschalten");
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str) -> DevicesStatus {
        DevicesStatus {
            enabled: state != "off",
            state: state.into(),
            mounted: state == "mounted",
            root_path: Some("/home/u/SmartExplorer/Geräte".into()),
            letter: None,
            mount_point: None,
            revision: 1,
            devices: Vec::new(),
            pending: 0,
        }
    }

    #[test]
    fn devices_task_settings_state_texts() {
        assert_eq!(state_text(&status("off"), None), "Aus");
        assert_eq!(state_text(&status("mounting"), None), "Wird eingebunden …");
        assert_eq!(
            state_text(&status("mounted"), None),
            "Eingebunden in /home/u/SmartExplorer/Geräte"
        );
        let mut windows = status("mounted");
        windows.letter = Some("X:".into());
        assert_eq!(state_text(&windows, None), "Eingebunden als X:");
        assert_eq!(state_text(&status("unmounting"), None), "Wird ausgehängt …");
        assert_eq!(
            state_text(&status("waiting"), None),
            "Wartet auf Smart Explorer Share"
        );
        assert_eq!(
            state_text(&status("failed"), Some("FUSE fehlt")),
            "Fehlgeschlagen: FUSE fehlt"
        );
        assert_eq!(state_text(&status("stopped"), None), "Angehalten");
        assert_eq!(
            pending_text(2),
            "Nicht gespeicherte Änderungen für 2 Gerät(e) warten auf eine Verbindung."
        );
    }
}
