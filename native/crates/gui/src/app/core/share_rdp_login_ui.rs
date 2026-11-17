//! The sign-in with the device's account on the device page (plan
//! 2026-11-17 P4, TODO RDP-ZIELKONTO): the state line, the one masked
//! dialog „Kennwort von <Konto> auf <Gerät>; wird auf <Gerät> gespeichert“
//! and „Hinterlegtes Kennwort löschen“. The password goes only to the
//! device over the authenticated share connection (`RdpStorePassword`),
//! which checks and stores it; this computer keeps no copy. The state comes
//! from the device (`RdpLogin` without password, no side effect).
use super::{Shared, State};
use crate::share::apps_wire::ManagedSecret;
use crate::share::ShareProfiles;
use eframe::egui;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

/// The state line is read again at most this often while the page shows
/// (a display rate: each read probes the device, on Windows with
/// PowerShell). Storing, forgetting and every start refresh it at once.
const STATUS_EVERY: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct Panel {
    /// The German state line from the device, or why it is unknown.
    status: Option<String>,
    /// The device keeps a password this computer typed.
    stored: bool,
    reading: bool,
    read_at: Option<Instant>,
    /// The start that waits for the password: its RemoteApp and the
    /// account the device names.
    pub(super) entry: Option<(Option<String>, String)>,
    password: String,
}

impl Panel {
    /// The answer of the device after a change.
    pub(super) fn note(&mut self, info: &crate::rdp::core::LoginInfo) {
        self.note_view(&info.view());
    }

    /// The sign-in of a start or an answer as surfaces show it.
    pub(super) fn note_view(&mut self, view: &crate::rdp::core::LoginView) {
        self.status = Some(view.text.clone());
        self.stored = view.state == "stored";
        self.read_at = Some(Instant::now());
    }
}

/// The name the profiles give the device, else its ID.
pub(super) fn device_name(profiles: &ShareProfiles, device: &str, key: &str) -> String {
    let direct = profiles.direct_contacts.iter().find(|contact| {
        contact.remote_device_id.as_deref() == Some(device)
            && contact.remote_public_key.as_deref() == Some(key)
    });
    if let Some(contact) = direct.filter(|contact| !contact.display_name.is_empty()) {
        return contact.display_name.clone();
    }
    profiles
        .rooms
        .iter()
        .flat_map(|room| &room.members)
        .find(|member| member.device_id == device && member.public_key == key)
        .map(|member| member.device_name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| device.to_string())
}

/// The state line and „Hinterlegtes Kennwort löschen“; reads the state in
/// the background while the device is present.
pub(super) fn status(
    ui: &mut egui::Ui,
    shared: &Shared,
    state: &mut State,
    (device, key): (&str, &str),
    present: bool,
) {
    let panel = &mut state.login;
    let due = panel.read_at.is_none_or(|at| at.elapsed() >= STATUS_EVERY);
    if present && due && !panel.reading && !state.busy {
        panel.reading = true;
        if let Err(error) = read(ui.ctx(), shared, device, key, None) {
            state.login.reading = false;
            state.error = Some(error);
        }
    }
    let panel = &state.login;
    if let Some(line) = &panel.status {
        ui.label(line);
    }
    if panel.stored
        && ui
            .add_enabled(
                present && !panel.reading && !state.busy,
                egui::Button::new("Hinterlegtes Kennwort löschen"),
            )
            .clicked()
    {
        state.login.reading = true;
        if let Err(error) = read(ui.ctx(), shared, device, key, Some(None)) {
            state.login.reading = false;
            state.error = Some(error);
        }
    }
}

/// Reads the device's state (`change` = `None`) or forgets the password
/// there first (`Some(None)`), in the background. The caller holds the
/// page's lock: a thread that cannot start is its error, not a second lock.
fn read(
    ctx: &egui::Context,
    shared: &Shared,
    device: &str,
    key: &str,
    change: Option<Option<ManagedSecret>>,
) -> Result<(), String> {
    let (state, context) = (shared.clone(), ctx.clone());
    let (device, key) = (device.to_string(), key.to_string());
    let spawned = std::thread::Builder::new()
        .name("device-rdp-login".into())
        .spawn(move || {
            use crate::rdp::viewer_login::{login_status, store_password};
            let result = match change {
                None => login_status(&device, &key, None),
                Some(password) => store_password(&device, &key, password, None),
            };
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.login.reading = false;
            match result {
                Ok(info) => state.login.note(&info),
                Err(error) => {
                    state.login.read_at = Some(Instant::now());
                    state.login.status = Some(format!(
                        "Anmeldung mit dem Konto des Geräts: Stand nicht lesbar ({error})"
                    ));
                }
            }
            context.request_repaint();
        });
    spawned
        .map(|_| ())
        .map_err(|error| format!("RDP-Anmeldestand nicht gelesen: {error}"))
}

/// What the dialog starts: its RemoteApp, the account the device named and
/// the typed password (`None`: start without storing; the RDP program asks).
pub(super) type Entered = (Option<String>, String, Option<ManagedSecret>);

/// The one masked dialog before a start whose device needs the password
/// once.
pub(super) fn entry_dialog(
    ui: &mut egui::Ui,
    panel: &mut Panel,
    device_name: &str,
) -> Option<Entered> {
    let (remote_app, account) = panel.entry.clone()?;
    ui.label(format!(
        "Kennwort von {account} auf {device_name}; wird auf {device_name} gespeichert"
    ));
    ui.add(
        egui::TextEdit::singleline(&mut panel.password)
            .password(true)
            .hint_text("Kennwort"),
    );
    let mut start = None;
    ui.horizontal_wrapped(|ui| {
        let typed = !panel.password.is_empty();
        if ui
            .add_enabled(typed, egui::Button::new("Speichern und verbinden"))
            .clicked()
        {
            let password = ManagedSecret::new(std::mem::take(&mut panel.password));
            start = Some((remote_app.clone(), account.clone(), Some(password)));
        }
        if ui.button("Ohne Speichern verbinden").clicked() {
            start = Some((remote_app.clone(), account.clone(), None));
        }
        if ui.button("Abbrechen").clicked() {
            panel.entry = None;
        }
    });
    if start.is_some() || panel.entry.is_none() {
        panel.password.clear();
        panel.entry = None;
    }
    start
}
