//! The window „Tresor-Eigenschaften“ (U29-7, T10, T11, T12/T13, T14): the
//! pages Allgemein, Sperren, Passwort, Wiederherstellung, Sync, Laufwerk and
//! Prüfung. Allgemein and Sperren share one draft and „Übernehmen“. The page
//! Wiederherstellung is text only: the word list of the recovery keys is not
//! free (C1.10), so there is no field and no button.
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;
use zeroize::Zeroize;

use super::vault_check_ui;
use super::vault_feed::{self, guard, poke, poll, spawn, Poll};
use super::vault_models::{recovery_page, RecordModel, VaultRow};
use super::vault_models_manage::{
    is_empty_change, password_change_step, password_done_text, settings_change, SettingsDraft,
};
use super::vault_settings_pages_ui::{DriveInfo, SyncInfo};
use super::vault_sidebar_ui::Effect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    General,
    Locks,
    Password,
    Recovery,
    Sync,
    Drive,
    Check,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::General,
        Page::Locks,
        Page::Password,
        Page::Recovery,
        Page::Sync,
        Page::Drive,
        Page::Check,
    ];

    fn title(self) -> &'static str {
        match self {
            Page::General => "Allgemein",
            Page::Locks => "Sperren",
            Page::Password => "Passwort",
            Page::Recovery => "Wiederherstellung",
            Page::Sync => "Sync",
            Page::Drive => "Laufwerk",
            Page::Check => "Prüfung",
        }
    }
}

#[derive(Default)]
struct PasswordDraft {
    old: String,
    new: String,
    repeat: String,
    rx: Option<Receiver<Result<String, String>>>,
    message: Option<String>,
    error: Option<String>,
}

struct Window {
    site: String,
    page: Page,
    /// The record the draft was made from.
    base: Option<RecordModel>,
    draft: Option<SettingsDraft>,
    apply_rx: Option<Receiver<Result<(), String>>>,
    message: Option<String>,
    error: Option<String>,
    password: PasswordDraft,
    sync: SyncInfo,
    drive: DriveInfo,
}

fn state() -> &'static Mutex<Option<Window>> {
    static STATE: OnceLock<Mutex<Option<Window>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// Opens the properties of `site`.
pub(crate) fn open(site: &str) {
    *guard(state()) = Some(Window {
        site: site.to_string(),
        page: Page::General,
        base: None,
        draft: None,
        apply_rx: None,
        message: None,
        error: None,
        password: PasswordDraft::default(),
        sync: SyncInfo::default(),
        drive: DriveInfo::default(),
    });
}

/// The site left the list: its window and its findings go.
pub(crate) fn close_site(site: &str) {
    let mut held = guard(state());
    if held.as_ref().is_some_and(|window| window.site == site) {
        *held = None;
    }
    drop(held);
    vault_check_ui::close_site(site);
}

fn resolve(location: &str) -> Result<(crate::vfs::BackendHandle, String), String> {
    crate::connect::resolve_endpoint(location)
}

fn update(site: String, change: crate::vault::SettingsChange) -> Result<(), String> {
    crate::vault::update_settings(
        &crate::vault::RegistryStore::open_default(),
        &site,
        &change,
        &crate::vault::OsSecretStore,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Draws the window.
pub(crate) fn show(ctx: &egui::Context) -> Vec<Effect> {
    let mut held = guard(state());
    let Some(window) = held.as_mut() else {
        return Vec::new();
    };
    let rows = vault_feed::rows();
    let Some(row) = rows.iter().find(|row| row.site == window.site).cloned() else {
        *held = None;
        return Vec::new();
    };
    let mut effects = Vec::new();
    refresh(window, &row);
    let mut open = true;
    egui::Window::new(format!("Tresor-Eigenschaften – {}", row.label))
        .id(egui::Id::new("vault_settings"))
        .open(&mut open)
        .collapsible(false)
        .default_size([620.0, 400.0])
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(120.0);
                    for page in Page::ALL {
                        ui.selectable_value(&mut window.page, page, page.title());
                    }
                });
                ui.separator();
                ui.vertical(|ui| page_ui(ui, window, &row, &rows, &mut effects));
            });
        });
    if !open {
        *held = None;
    }
    effects
}

/// Takes the answers of the threads and renews the draft when the vault
/// list changed under it.
fn refresh(window: &mut Window, row: &VaultRow) {
    match poll(&mut window.apply_rx) {
        Poll::Ready(Ok(())) => {
            window.message = Some("Gespeichert.".to_string());
            window.error = None;
            poke();
        }
        Poll::Ready(Err(error)) => window.error = Some(error),
        Poll::Gone => window.error = Some("Das Speichern endete ohne Antwort.".to_string()),
        Poll::Pending => {}
    }
    match poll(&mut window.password.rx) {
        Poll::Ready(Ok(text)) => {
            window.password.message = Some(text);
            window.password.error = None;
        }
        Poll::Ready(Err(error)) => {
            window.password.error = Some(error);
            window.password.message = None;
        }
        Poll::Gone => {
            window.password.error = Some("Die Änderung endete ohne Antwort.".to_string());
        }
        Poll::Pending => {}
    }
    if window.base.as_ref() != Some(&row.record) {
        window.draft = Some(SettingsDraft::from_record(&row.record));
        window.base = Some(row.record.clone());
    }
}

fn page_ui(
    ui: &mut egui::Ui,
    window: &mut Window,
    row: &VaultRow,
    rows: &[VaultRow],
    effects: &mut Vec<Effect>,
) {
    let unlocked = row.state == super::vault_models::RowState::Unlocked;
    match window.page {
        Page::General | Page::Locks => settings_page(ui, window, rows),
        Page::Password => password_page(ui, window, row),
        Page::Recovery => {
            for paragraph in recovery_page().paragraphs {
                ui.label(paragraph);
                ui.add_space(4.0);
            }
        }
        Page::Sync => window.sync.show(ui, &window.site, effects),
        Page::Drive => {
            let running = vault_feed::status().is_some();
            window
                .drive
                .show(ui, &window.site, &row.label, unlocked, running, effects);
        }
        Page::Check => effects.extend(vault_check_ui::page(ui, &window.site, unlocked)),
    }
}

fn settings_page(ui: &mut egui::Ui, window: &mut Window, rows: &[VaultRow]) {
    let (Some(draft), Some(base)) = (window.draft.as_mut(), window.base.as_ref()) else {
        return;
    };
    let status = vault_feed::status();
    if window.page == Page::General {
        ui.label("Bezeichnung");
        ui.add(egui::TextEdit::singleline(&mut draft.label).desired_width(280.0));
        ui.label(format!("Ort: {}", base.location));
        let kind = match &base.replica_of {
            Some(original) => {
                let name = rows
                    .iter()
                    .find(|other| &other.site == original)
                    .map_or_else(|| original.clone(), |other| other.label.clone());
                format!("Kopie von {name}")
            }
            None => "Original".to_string(),
        };
        ui.label(format!("Art: {kind}"));
        let short: String = base.vault_id.chars().take(8).collect();
        ui.label(format!("Tresorkennung: {short}"));
        ui.separator();
        let remembered = base.remember_password;
        let mut shown = remembered;
        let switched = ui
            .add_enabled(
                remembered,
                egui::Checkbox::new(&mut shown, "Passwort merken"),
            )
            .on_hover_text("Eingeschaltet wird es beim Entsperren („Passwort merken“).")
            .changed();
        let forget = ui
            .add_enabled(
                remembered,
                egui::Button::new("Gemerktes Passwort vergessen"),
            )
            .clicked();
        if (switched && !shown) || forget {
            let change = crate::vault::SettingsChange {
                remember_password: Some(false),
                ..crate::vault::SettingsChange::default()
            };
            start_apply(ui.ctx(), window, change);
            return;
        }
    } else {
        ui.checkbox(&mut draft.idle_on, "Nach Leerlauf sperren");
        ui.add_enabled_ui(draft.idle_on, |ui| {
            ui.horizontal(|ui| {
                ui.label("nach");
                ui.add(egui::TextEdit::singleline(&mut draft.idle_text).desired_width(50.0));
                ui.label("Minuten");
            });
        });
        let sleep_ok = status.as_ref().is_none_or(|status| status.sleep_lock);
        ui.add_enabled(
            sleep_ok,
            egui::Checkbox::new(&mut draft.lock_on_sleep, "Beim Ruhezustand sperren"),
        );
        if !sleep_ok {
            ui.small("Dieses System meldet das Signal nicht.");
        }
        let screen_ok = status.as_ref().is_none_or(|status| status.screen_lock);
        ui.add_enabled(
            screen_ok,
            egui::Checkbox::new(
                &mut draft.lock_on_screen_lock,
                "Bei Bildschirmsperre sperren",
            ),
        );
        if !screen_ok {
            ui.small("Dieses System meldet das Signal nicht.");
        }
        if let Some(note) = status
            .as_ref()
            .and_then(|status| status.screen_lock_note.as_ref())
        {
            ui.small(note);
        }
    }
    ui.separator();
    let change = settings_change(draft, base);
    let busy = window.apply_rx.is_some();
    let pending = matches!(&change, Ok(change) if !is_empty_change(change));
    if let Err(issue) = &change {
        ui.colored_label(crate::app::theme::danger(ui), issue);
    }
    if let Some(error) = &window.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    if let Some(message) = &window.message {
        ui.small(message);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(pending && !busy, egui::Button::new("Übernehmen"))
            .clicked()
        {
            if let Ok(change) = change {
                start_apply(ui.ctx(), window, change);
            }
        }
        if busy {
            ui.spinner();
        }
    });
}

fn start_apply(ctx: &egui::Context, window: &mut Window, change: crate::vault::SettingsChange) {
    window.message = None;
    window.error = None;
    let site = window.site.clone();
    match spawn(ctx, "vault-settings", move || update(site, change)) {
        Ok(rx) => window.apply_rx = Some(rx),
        Err(error) => window.error = Some(error),
    }
}

fn password_page(ui: &mut egui::Ui, window: &mut Window, row: &VaultRow) {
    let ctx = ui.ctx().clone();
    let draft = &mut window.password;
    let busy = draft.rx.is_some();
    for (label, field) in [
        ("Altes Passwort", &mut draft.old),
        ("Neues Passwort", &mut draft.new),
        ("Neues Passwort wiederholen", &mut draft.repeat),
    ] {
        ui.label(label);
        ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(field)
                .password(true)
                .desired_width(280.0),
        );
    }
    let check = password_change_step(&draft.old, &draft.new, &draft.repeat);
    if let (Err(issue), true) = (&check, !draft.new.is_empty()) {
        ui.colored_label(crate::app::theme::danger(ui), issue);
    }
    if let Some(error) = &draft.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    if let Some(message) = &draft.message {
        ui.label(message);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy && check.is_ok(), egui::Button::new("Passwort ändern"))
            .clicked()
        {
            let site = row.site.clone();
            let (mut old, mut new) = (
                std::mem::take(&mut draft.old),
                std::mem::take(&mut draft.new),
            );
            draft.repeat.zeroize();
            draft.message = None;
            draft.error = None;
            match spawn(&ctx, "vault-password", move || {
                let changed = crate::vault::change_site_password(
                    &crate::vault::RegistryStore::open_default(),
                    &crate::vault::OsSecretStore,
                    &site,
                    &old,
                    &new,
                    crate::vault::available_memory(),
                    &resolve,
                );
                old.zeroize();
                new.zeroize();
                match changed {
                    Ok(changed) => Ok(password_done_text(
                        &changed.backup,
                        changed.remember_error.as_deref(),
                    )),
                    Err(error) => Err(error
                        .code()
                        .and_then(crate::vault::error_text_for_code)
                        .map_or_else(|| error.to_string(), str::to_string)),
                }
            }) {
                Ok(rx) => draft.rx = Some(rx),
                Err(error) => draft.error = Some(error),
            }
        }
        if busy {
            ui.spinner();
            ui.label("Das Passwort wird geändert …");
        }
    });
}
