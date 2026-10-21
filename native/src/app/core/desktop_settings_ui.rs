//! Einstellungen → Integration → „se Desktop“ (DSK-M3; V14: the last section
//! of the page): state, parts with the reason when one is unavailable, the
//! desktop layer, the login start, the buttons Aktivieren, Deaktivieren,
//! Entfernen… (confirmed) and Neu starten, the traces left after a removal,
//! and the hint of C10 with the existing background autostart. Status and
//! actions run in a thread, so the page never waits for them. Also the start
//! pages of S5 windows (`--open-location`, `--open-page`, V9).
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::desktop_settings_gnome_ui as gnome_ui;
use super::desktop_surface_settings_ui::ui_desktop_appearance;
use crate::app::prelude::*;
use crate::app::settings_ui::SettingsPage;
use crate::app::start_request::{StartPage, StartRequest};
use crate::app::{theme, App};
use crate::desktop::core::state::{Part, State};
use crate::desktop::os::control::{AutostartState, Outcome};
use crate::desktop::os::launch::with_control;
use crate::desktop::os::status::StatusReport;

/// How old the shown state may get while the page is open.
const REFRESH: Duration = Duration::from_secs(3);
/// C10: the sentence and the one button of the existing autostart.
const LIVE_HINT: &str = "Live-Gerätezustand ab Anmeldung: Autostart des Hintergrunddienstes \
einschalten (macht dieses Gerät ab Anmeldung für gekoppelte Geräte erreichbar)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Enable,
    Disable,
    Remove,
    Restart,
    Part(Part, bool),
    /// GNOME: let se Desktop take over the desktop icons of DING (U34-17).
    TakeOver,
}

#[derive(Default)]
struct Panel {
    report: Option<StatusReport>,
    loading: bool,
    loaded_at: Option<Instant>,
    busy: Option<Action>,
    message: Option<Result<String, String>>,
    confirm_remove: bool,
}

type Shared = Arc<Mutex<Panel>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Panel> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn refresh(shared: &Shared, ctx: &egui::Context) {
    {
        let mut panel = lock(shared);
        let fresh = panel.loaded_at.is_some_and(|at| at.elapsed() < REFRESH);
        if panel.loading || fresh {
            return;
        }
        panel.loading = true;
    }
    let worker = shared.clone();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("se-desktop-status".into())
        .spawn(move || {
            let report = crate::desktop::os::status_report();
            let mut panel = lock(&worker);
            panel.report = Some(report);
            panel.loading = false;
            panel.loaded_at = Some(Instant::now());
            drop(panel);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        lock(shared).loading = false;
    }
}

fn outcome_text(done: &str, outcome: &Outcome) -> Result<String, String> {
    let mut problems: Vec<String> = outcome.errors.clone();
    for trace in &outcome.residue {
        problems.push(format!("Restspur {} {}", trace.kind.key(), trace.target));
    }
    for file in &outcome.residue_files {
        problems.push(format!("Restspur {}", file.display()));
    }
    if problems.is_empty() {
        Ok(done.to_string())
    } else {
        Err(format!("{done}, aber: {}", problems.join("; ")))
    }
}

fn perform(action: Action) -> Result<String, String> {
    with_control(|control| match action {
        Action::Enable => control
            .enable(None)
            .map_err(|error| error.to_string())
            .and_then(|outcome| outcome_text("se Desktop ist aktiviert", &outcome)),
        Action::Disable => control
            .disable()
            .map_err(|error| error.to_string())
            .and_then(|outcome| outcome_text("se Desktop ist deaktiviert", &outcome)),
        Action::Remove => control
            .remove()
            .map_err(|error| error.to_string())
            .and_then(|outcome| outcome_text("se Desktop ist entfernt", &outcome)),
        Action::Restart => match control.restart() {
            Ok(outcome) if outcome.restarted => Ok("se Desktop wurde neu gestartet".into()),
            Ok(outcome) => Err(outcome
                .reason
                .unwrap_or_else(|| "nicht neu gestartet".into())),
            Err(error) => Err(error.to_string()),
        },
        Action::Part(part, on) => control
            .set_part(part, on)
            .map_err(|error| error.to_string())
            .and_then(|outcome| outcome_text("Teil geändert", &outcome)),
        Action::TakeOver => gnome_ui::take_over(control),
    })
}

fn start(shared: &Shared, ctx: &egui::Context, action: Action) {
    {
        let mut panel = lock(shared);
        if panel.busy.is_some() {
            return;
        }
        panel.busy = Some(action);
        panel.message = None;
        panel.confirm_remove = false;
    }
    let worker = shared.clone();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("se-desktop-action".into())
        .spawn(move || {
            let result = perform(action);
            let mut panel = lock(&worker);
            panel.busy = None;
            panel.message = Some(result);
            panel.loaded_at = None;
            drop(panel);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        let mut panel = lock(shared);
        panel.busy = None;
        panel.message = Some(Err("Hintergrundvorgang konnte nicht starten".into()));
    }
}

fn state_label(state: State) -> &'static str {
    match state {
        State::Absent => "nicht hinzugefügt",
        State::Enabled => "aktiviert",
        State::Disabled => "deaktiviert",
    }
}

fn part_label(part: Part) -> &'static str {
    match part {
        Part::Surface => "Desktopfläche",
        Part::NavPane => "„Smart Explorer Geräte“ im Explorer-Navigationsbereich",
        Part::JumpList => "Sprungliste „Geräte“ am Taskleistensymbol",
    }
}

fn autostart_label(autostart: AutostartState) -> &'static str {
    match autostart {
        AutostartState::On => "an",
        AutostartState::Off => "aus",
        AutostartState::DisabledBySystem => "vom System abgeschaltet (wird nicht überschrieben)",
    }
}

impl App {
    /// The last section of Einstellungen → Integration.
    pub(in crate::app) fn ui_desktop_settings(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "se Desktop");
        ui.label(
            RichText::new(
                "Eigene Desktopfläche mit Geräten, Räumen und Tresoren; Deaktivieren und \
                 Entfernen hinterlassen keine Spuren.",
            )
            .color(theme::muted(ui)),
        );
        let shared: Shared = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<Shared>(egui::Id::new("se_desktop_settings"))
                .clone()
        });
        refresh(&shared, ui.ctx());
        let (report, busy, message, confirm) = {
            let panel = lock(&shared);
            (
                panel.report.clone(),
                panel.busy,
                panel.message.clone(),
                panel.confirm_remove,
            )
        };
        let Some(report) = report else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Zustand wird gelesen …");
            });
            return;
        };
        self.ui_desktop_state(ui, &shared, &report, busy, confirm);
        if report.state != State::Absent {
            ui_desktop_parts(ui, &shared, &report, busy);
            // DSK-M4: background, groups and "Geräte öffnen mit".
            ui_desktop_appearance(ui, &report);
        }
        ui_desktop_facts(ui, &report);
        if !report.daemon_autostart {
            self.ui_desktop_live_hint(ui);
        }
        match message {
            Some(Ok(text)) => {
                ui.colored_label(theme::success(ui), text);
            }
            Some(Err(text)) => {
                ui.colored_label(theme::danger(ui), text);
            }
            None => {}
        }
    }

    fn ui_desktop_state(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Shared,
        report: &StatusReport,
        busy: Option<Action>,
        confirm: bool,
    ) {
        ui.label(format!("Zustand: {}", state_label(report.state)));
        if let Some(error) = &report.state_error {
            ui.colored_label(theme::danger(ui), error.as_str());
        }
        if let Some(halt) = &report.halted {
            ui.colored_label(
                theme::warning(ui),
                format!(
                    "Nach wiederholten Abstürzen angehalten (Version {}); „Neu starten“ \
                     versucht es erneut.",
                    halt.version
                ),
            );
        }
        let idle = busy.is_none();
        ui.horizontal_wrapped(|ui| {
            if report.state != State::Enabled
                && ui
                    .add_enabled(idle, egui::Button::new("Aktivieren"))
                    .clicked()
            {
                start(shared, ui.ctx(), Action::Enable);
            }
            if report.state == State::Enabled {
                if ui
                    .add_enabled(idle, egui::Button::new("Deaktivieren"))
                    .clicked()
                {
                    start(shared, ui.ctx(), Action::Disable);
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Neu starten"))
                    .clicked()
                {
                    start(shared, ui.ctx(), Action::Restart);
                }
            }
            if report.state != State::Absent
                && ui
                    .add_enabled(idle, egui::Button::new("Entfernen …"))
                    .clicked()
            {
                lock(shared).confirm_remove = true;
            }
            if busy.is_some() {
                ui.spinner();
            }
        });
        if confirm {
            ui.colored_label(
                theme::warning(ui),
                "Entfernen beendet se Desktop und löscht jede Spur samt Einstellungen und \
                 Anordnung. Der Gerätebaum „Verbundene Geräte als Ordner“ bleibt.",
            );
            ui.horizontal(|ui| {
                if ui.button("Entfernen").clicked() {
                    start(shared, ui.ctx(), Action::Remove);
                }
                if ui.button("Abbrechen").clicked() {
                    lock(shared).confirm_remove = false;
                }
            });
        }
    }

    /// C10: the existing autostart of the background service, unchanged.
    fn ui_desktop_live_hint(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(LIVE_HINT).color(theme::muted(ui)));
        if !ui.button("Autostart einschalten").clicked() {
            return;
        }
        match crate::autostart::enable() {
            Ok(()) => match crate::daemon::request_daemon_replacement() {
                Ok(()) => {
                    self.notice = Some((
                        "✓ Hintergrund-Sync aktiviert".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    let rollback = crate::autostart::disable()
                        .err()
                        .map(|rollback| format!("; Autostart-Rücknahme: {rollback}"))
                        .unwrap_or_default();
                    self.error_msg = Some(format!(
                        "Hintergrund-Sync bleibt aus: Dienst konnte nicht sicher gestartet \
                         werden: {error}{rollback}"
                    ));
                }
            },
            Err(error) => self.error_msg = Some(format!("Autostart: {error}")),
        }
    }

    /// V9 window kinds of S5, once in the first frame. Tab numbers 0
    /// (Geräte) and 1 (Räume) keep their meaning for outside callers
    /// (`share_pages_ui.rs`). A device or room page opens through the page
    /// functions of `share_pages_ui.rs`; an unknown ID shows its tab.
    pub(in crate::app) fn open_start_request(&mut self, request: StartRequest) {
        match request {
            StartRequest::OpenLocation { locator } => self.navigate_to_location(&locator),
            StartRequest::OpenPage { page } => match page {
                StartPage::ShareDevices => {
                    self.show_share = true;
                    self.share_tab = 0;
                }
                StartPage::ShareDevice { contact_id } => {
                    self.open_share_contact_page(&contact_id);
                }
                StartPage::ShareRooms => {
                    self.show_share = true;
                    self.share_tab = 1;
                }
                StartPage::ShareRoom { room_id } => {
                    self.open_share_room_page(&room_id);
                }
                StartPage::SettingsIntegration => self.open_settings(SettingsPage::Integration),
            },
            _ => {}
        }
    }
}

fn ui_desktop_parts(
    ui: &mut egui::Ui,
    shared: &Shared,
    report: &StatusReport,
    busy: Option<Action>,
) {
    for part in &report.parts {
        if !part.supported {
            continue;
        }
        let available = part.unavailable.is_none();
        let mut on = part.enabled;
        let toggled = ui
            .add_enabled(
                busy.is_none() && (available || on),
                egui::Checkbox::new(&mut on, part_label(part.part)),
            )
            .changed();
        if toggled {
            start(shared, ui.ctx(), Action::Part(part.part, on));
        }
        if let Some(reason) = &part.reason {
            let shown = if available {
                reason.clone()
            } else {
                format!("nicht verfügbar: {reason}")
            };
            ui.label(RichText::new(shown).small().color(theme::muted(ui)));
        }
        if part.part == Part::Surface && gnome_ui::ui_gnome(ui, report, part, busy.is_none()) {
            start(shared, ui.ctx(), Action::TakeOver);
        }
    }
}

fn ui_desktop_facts(ui: &mut egui::Ui, report: &StatusReport) {
    let muted = theme::muted(ui);
    let mechanism = match &report.mechanism {
        crate::desktop::core::mechanism::Mechanism::Unavailable(reason) => {
            format!("Desktop-Ebene: nicht verfügbar – {}", reason.text())
        }
        mechanism => format!("Desktop-Ebene: {}", mechanism.key()),
    };
    ui.label(RichText::new(mechanism).small().color(muted));
    ui.label(
        RichText::new(format!(
            "Start bei der Anmeldung: {} · Flächen: {} von {}",
            autostart_label(report.autostart),
            report.surfaces.len(),
            report.monitors
        ))
        .small()
        .color(muted),
    );
    if !report.residue.is_empty() {
        let list: Vec<String> = report
            .residue
            .iter()
            .map(|(kind, target)| format!("{kind} {target}"))
            .collect();
        ui.colored_label(
            theme::warning(ui),
            format!("Restspuren: {}", list.join(", ")),
        );
    }
}

#[cfg(test)]
#[path = "desktop_settings_ui_tests.rs"]
mod tests;
