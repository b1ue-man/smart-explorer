//! The creation wizard „Neuer Tresor“ (U29-6); „Tresor hinzufügen…“ is in
//! `vault_add_ui.rs`, the location field is shared. The wizard has three steps (location, name and password,
//! summary) and leaves the new vault locked: the password is typed only for
//! the creation, an unlock would be a second key derivation. Passwords stay
//! in masked fields and are wiped after use; nothing of them goes to a log.
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;
use zeroize::Zeroize;

use super::vault_feed::{guard, poke, poll, spawn, Poll};
use super::vault_models_manage::{
    create_location_issue, default_label, label_issue, location_view, parse_threshold,
    password_step, LocationView, THRESHOLD_DEFAULT, THRESHOLD_MAX, THRESHOLD_MIN,
};
use super::vault_sidebar_ui::{Effect, PickTarget};
use crate::creds::SavedConnection;

pub(super) type Checked = Result<LocationView, String>;

/// The chooser of a location: the field, the saved connections.
#[derive(Default)]
pub(super) struct Chooser {
    pub(super) connections_open: bool,
    pub(super) index: usize,
    pub(super) path: String,
}

struct Wizard {
    step: u8,
    location: String,
    chooser: Chooser,
    label: String,
    label_touched: bool,
    password: String,
    repeat: String,
    threshold: String,
    check_rx: Option<Receiver<Checked>>,
    warning: Option<String>,
    error: Option<String>,
    create_rx: Option<Receiver<Result<String, String>>>,
}

#[derive(Default)]
struct State {
    wizard: Option<Wizard>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

pub(super) fn resolve(location: &str) -> Result<(crate::vfs::BackendHandle, String), String> {
    crate::connect::resolve_endpoint(location)
}

pub(super) fn check_location(location: String) -> Checked {
    crate::vault::detect_location(location.trim(), &resolve)
        .map(|found| location_view(&found))
        .map_err(|error| error.to_string())
}

/// Opens the creation wizard.
pub(crate) fn open_create() {
    guard(state()).wizard = Some(Wizard {
        step: 1,
        location: String::new(),
        chooser: Chooser::default(),
        label: String::new(),
        label_touched: false,
        password: String::new(),
        repeat: String::new(),
        threshold: THRESHOLD_DEFAULT.to_string(),
        check_rx: None,
        warning: None,
        error: None,
        create_rx: None,
    });
}

/// A folder picked for the wizard (`true`) or the add window (`false`).
pub(crate) fn picked(wizard: bool, folder: String) {
    if !wizard {
        super::vault_add_ui::picked(folder);
        return;
    }
    if let Some(wizard) = guard(state()).wizard.as_mut() {
        wizard.location = folder;
    }
}

/// Opens „Tresor hinzufügen…“ (the window is in `vault_add_ui.rs`).
pub(crate) fn open_add(prefill: Option<String>) {
    super::vault_add_ui::open(prefill);
}

/// The field of a location with its two ways to fill it.
pub(super) fn location_row(
    ui: &mut egui::Ui,
    location: &mut String,
    chooser: &mut Chooser,
    connections: &[SavedConnection],
    target: PickTarget,
    effects: &mut Vec<Effect>,
    enabled: bool,
) {
    ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(location)
            .hint_text("Ordner oder Adresse des Tresors")
            .desired_width(360.0),
    );
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(enabled, egui::Button::new("Lokaler Ordner …"))
            .clicked()
        {
            effects.push(Effect::PickFolder(target));
        }
        if ui
            .add_enabled(
                enabled && !connections.is_empty(),
                egui::Button::new("Gespeicherte Verbindung …"),
            )
            .clicked()
        {
            chooser.connections_open = !chooser.connections_open;
        }
    });
    if !chooser.connections_open || connections.is_empty() {
        return;
    }
    chooser.index = chooser.index.min(connections.len() - 1);
    egui::ComboBox::from_id_salt(("vault_connection", target as u8))
        .selected_text(connections[chooser.index].display())
        .show_ui(ui, |ui| {
            for (index, connection) in connections.iter().enumerate() {
                ui.selectable_value(&mut chooser.index, index, connection.display());
            }
        });
    ui.horizontal(|ui| {
        ui.label("Ordner auf der Verbindung");
        ui.add(egui::TextEdit::singleline(&mut chooser.path).desired_width(200.0));
        if ui.button("Übernehmen").clicked() {
            *location = connection_location(&connections[chooser.index], &chooser.path);
            chooser.connections_open = false;
        }
    });
}

/// The locator of a folder on a saved connection.
fn connection_location(connection: &SavedConnection, path: &str) -> String {
    if connection.protocol.is_url() {
        return crate::connect::remote_endpoint(connection, path.trim());
    }
    let base = connection.to_target();
    let path = path.trim().trim_start_matches(['/', '\\']);
    if path.is_empty() {
        base
    } else {
        format!("{}/{path}", base.trim_end_matches(['/', '\\']))
    }
}

/// Draws the windows and collects the answers of the threads.
pub(crate) fn show(ctx: &egui::Context, connections: &[SavedConnection]) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut held = guard(state());
    show_wizard(ctx, &mut held, connections, &mut effects);
    drop(held);
    effects.extend(super::vault_add_ui::show(ctx, connections));
    effects
}

fn show_wizard(
    ctx: &egui::Context,
    state: &mut State,
    connections: &[SavedConnection],
    effects: &mut Vec<Effect>,
) {
    let Some(wizard) = state.wizard.as_mut() else {
        return;
    };
    let mut finished = false;
    if let Poll::Ready(found) = poll(&mut wizard.check_rx) {
        wizard.warning = None;
        match found {
            Ok(found) => match create_location_issue(&found) {
                Some(issue) => wizard.error = Some(issue),
                None => {
                    wizard.error = None;
                    wizard.step = 2;
                }
            },
            // A new folder cannot be read yet; the creation says what is wrong.
            Err(problem) => {
                wizard.warning = Some(format!("Der Ort konnte nicht geprüft werden: {problem}"));
                wizard.error = None;
                wizard.step = 2;
            }
        }
        if wizard.step == 2 && !wizard.label_touched {
            wizard.label = default_label(&wizard.location);
        }
    }
    match poll(&mut wizard.create_rx) {
        Poll::Ready(Ok(_)) => {
            effects.push(Effect::Notice(
                "Tresor angelegt. Zum Öffnen entsperren.".to_string(),
            ));
            finished = true;
        }
        Poll::Ready(Err(error)) => {
            wizard.error = Some(error);
            wizard.step = 3;
        }
        Poll::Gone => {
            wizard.error = Some("Das Anlegen endete ohne Antwort.".to_string());
        }
        Poll::Pending => {}
    }
    if finished {
        poke();
        state.wizard = None;
        return;
    }
    let busy = wizard.check_rx.is_some() || wizard.create_rx.is_some();
    let (mut next, mut back, mut create, mut cancel) = (false, false, false, false);
    crate::app::dialog_geometry::window(ctx, "vault_create_ui.0", "Neuer Tresor")
        .id(egui::Id::new("vault_create_wizard"))
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(format!("Schritt {} von 3", wizard.step));
            ui.separator();
            let step_ok = match wizard.step {
                1 => {
                    ui.label("Wo soll der Tresor liegen?");
                    location_row(
                        ui,
                        &mut wizard.location,
                        &mut wizard.chooser,
                        connections,
                        PickTarget::CreateLocation,
                        effects,
                        !busy,
                    );
                    !wizard.location.trim().is_empty()
                }
                2 => {
                    ui.label("Bezeichnung");
                    if ui
                        .add(egui::TextEdit::singleline(&mut wizard.label).desired_width(280.0))
                        .changed()
                    {
                        wizard.label_touched = true;
                    }
                    ui.label("Passwort");
                    ui.add(
                        egui::TextEdit::singleline(&mut wizard.password)
                            .password(true)
                            .desired_width(280.0),
                    );
                    ui.label("Passwort wiederholen");
                    ui.add(
                        egui::TextEdit::singleline(&mut wizard.repeat)
                            .password(true)
                            .desired_width(280.0),
                    );
                    egui::CollapsingHeader::new("Erweitert")
                        .id_salt("vault_create_advanced")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Dateinamen kürzen ab");
                                ui.add(
                                    egui::TextEdit::singleline(&mut wizard.threshold)
                                        .desired_width(60.0),
                                );
                            });
                            ui.small(format!(
                                "Längere verschlüsselte Namen werden gekürzt gespeichert. \
                                 Zahl von {THRESHOLD_MIN} bis {THRESHOLD_MAX}."
                            ));
                        });
                    ui.small(format!("{}.", crate::vault::CREATE_RECOVERY_HINT));
                    let issue = label_issue(&wizard.label)
                        .or_else(|| password_step(&wizard.password, &wizard.repeat).err())
                        .or_else(|| parse_threshold(&wizard.threshold).err());
                    if let Some(issue) = &issue {
                        ui.colored_label(crate::app::theme::danger(ui), issue);
                    }
                    issue.is_none()
                }
                _ => {
                    ui.label(format!("Ort: {}", wizard.location.trim()));
                    ui.label(format!("Bezeichnung: {}", wizard.label.trim()));
                    ui.small("Der Tresor bleibt nach dem Anlegen gesperrt.");
                    true
                }
            };
            if let Some(warning) = &wizard.warning {
                ui.colored_label(crate::app::theme::warning(ui), warning);
            }
            if let Some(error) = &wizard.error {
                ui.colored_label(crate::app::theme::danger(ui), error);
            }
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if wizard.create_rx.is_some() {
                        "Tresor wird angelegt …"
                    } else {
                        "Ort wird geprüft …"
                    });
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                if wizard.step > 1 && ui.add_enabled(!busy, egui::Button::new("Zurück")).clicked()
                {
                    back = true;
                }
                if wizard.step < 3 {
                    if ui
                        .add_enabled(!busy && step_ok, egui::Button::new("Weiter"))
                        .clicked()
                    {
                        next = true;
                    }
                } else if ui
                    .add_enabled(!busy, egui::Button::new("Tresor anlegen"))
                    .clicked()
                {
                    create = true;
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Abbrechen"))
                    .clicked()
                {
                    cancel = true;
                }
            });
        });
    if cancel {
        wizard.password.zeroize();
        wizard.repeat.zeroize();
        state.wizard = None;
        return;
    }
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    if back {
        wizard.step -= 1;
        wizard.error = None;
    }
    if next && wizard.step == 1 {
        wizard.error = None;
        let location = wizard.location.clone();
        match spawn(ctx, "vault-detect", move || check_location(location)) {
            Ok(rx) => wizard.check_rx = Some(rx),
            Err(error) => wizard.error = Some(error),
        }
    } else if next {
        wizard.error = None;
        wizard.step += 1;
    }
    if create {
        start_create(ctx, wizard);
    }
}

fn start_create(ctx: &egui::Context, wizard: &mut Wizard) {
    wizard.error = None;
    let Ok(threshold) = parse_threshold(&wizard.threshold) else {
        wizard.error = Some("„Dateinamen kürzen ab“ ist keine gültige Zahl.".to_string());
        return;
    };
    let location = wizard.location.trim().to_string();
    let label = wizard.label.trim().to_string();
    let mut password = std::mem::take(&mut wizard.password);
    wizard.repeat.zeroize();
    let started = spawn(ctx, "vault-create", move || {
        let created = crate::vault::create_site(
            &crate::vault::RegistryStore::open_default(),
            &location,
            Some(&label),
            &password,
            threshold,
            crate::vault::available_memory(),
            &resolve,
        );
        password.zeroize();
        created
            .map(|record| record.display_label().to_string())
            .map_err(|error| error.to_string())
    });
    match started {
        Ok(rx) => wizard.create_rx = Some(rx),
        Err(error) => {
            wizard.error = Some(error);
            wizard.step = 2;
        }
    }
}
