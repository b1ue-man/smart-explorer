//! Device-scoped application picker; independent from Exec and screen capture.
use crate::{
    apps::{AppId, CatalogSnapshot, LaunchReceipt, LaunchRequest, LaunchState, WindowBinding},
    share::{PeerOpenTarget, ShareProfiles},
};
use eframe::egui;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};
#[path = "share_apps_jobs.rs"]
mod jobs;
#[path = "share_apps_store.rs"]
mod store;

enum Icon {
    Loading,
    Ready(egui::TextureHandle),
    Unavailable(String),
}
#[derive(Default)]
struct State {
    restored: bool,
    busy: bool,
    pending: Option<String>,
    error: Option<String>,
    catalog: Option<CatalogSnapshot>,
    saved: Option<store::Saved>,
    receipt: Option<LaunchReceipt>,
    node: Option<String>,
    search: String,
    page: usize,
    icons: BTreeMap<AppId, Icon>,
    icons_busy: bool,
    new_request_ack: bool,
}
type Shared = Arc<Mutex<State>>;
struct Choice {
    target: PeerOpenTarget,
    label: String,
}
#[derive(Clone)]
struct Surface {
    target: PeerOpenTarget,
    label: String,
}

/// Constructed only from a matching live authenticated daemon receipt. An
/// opener must still obtain P4's current generation-bound selection evidence.
#[derive(Clone)]
pub(crate) struct ConfirmedAppWindow {
    pub(crate) target: PeerOpenTarget,
    pub(crate) node_id: String,
    pub(crate) request: LaunchRequest,
    pub(crate) window: WindowBinding,
}
type WindowOpener = fn(&egui::Context, ConfirmedAppWindow) -> Result<(), String>;
static OPENER: OnceLock<WindowOpener> = OnceLock::new();
/// Every GUI process installs the same P4 resolver, including standalone surfaces.
pub(crate) fn install_window_opener() {
    OPENER.get_or_init(|| super::family_picker::open_confirmed);
}
pub(crate) fn device(ui: &mut egui::Ui, profiles: &ShareProfiles, device: &str, key: &str) {
    let mut choices = Vec::new();
    for contact in &profiles.direct_contacts {
        if contact.remote_device_id.as_deref() == Some(device)
            && contact.remote_public_key.as_deref() == Some(key)
        {
            choices.push(Choice {
                target: PeerOpenTarget::Direct {
                    contact_id: contact.id.clone(),
                },
                label: format!("Direkt: {}", contact.display_name),
            });
        }
    }
    for room in &profiles.rooms {
        for member in &room.members {
            if member.device_id == device && member.public_key == key && !member.blocked {
                choices.push(Choice {
                    target: PeerOpenTarget::RoomDevice {
                        room_id: room.room_id.clone(),
                        device_id: device.into(),
                    },
                    label: format!("Raum „{}“ / {}", room.name, member.device_name),
                });
            }
        }
    }
    if choices.is_empty() {
        ui.label("Keine gespeicherte ausgehende Beziehung zu dieser bestätigten Geräteidentität.");
        return;
    }
    let id = egui::Id::new(("apps-relation", device, key));
    let mut selected = ui
        .ctx()
        .data_mut(|d| *d.get_temp_mut_or_insert_with(id, || 0usize))
        .min(choices.len() - 1);
    let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
    super::device_page_layout::relation_picker(ui, &labels, &mut selected);
    ui.ctx().data_mut(|d| d.insert_temp(id, selected));
    panel(ui, &choices[selected].target, Some(device));
}
pub(crate) fn open_surface(ctx: &egui::Context, target: PeerOpenTarget, label: String) {
    let id = egui::Id::new("device-apps-windows");
    ctx.data_mut(|d| {
        let windows = d.get_temp_mut_or_insert_with(id, Vec::<Surface>::new);
        if !windows.iter().any(|w| w.target == target) {
            windows.push(Surface { target, label });
        }
    });
    ctx.request_repaint();
}
pub(crate) fn surface_windows(ctx: &egui::Context) {
    let id = egui::Id::new("device-apps-windows");
    let windows = ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, Vec::<Surface>::new)
            .clone()
    });
    let mut closed = Vec::new();
    for window in windows {
        let mut open = true;
        egui::Window::new(format!("Programme · {}", window.label))
            .id(id.with(window.target.endpoint_prefix()))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                panel(ui, &window.target, None);
            });
        if !open {
            closed.push(window.target);
        }
    }
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, Vec::<Surface>::new)
            .retain(|w| !closed.contains(&w.target));
    });
}
fn panel(ui: &mut egui::Ui, target: &PeerOpenTarget, expected: Option<&str>) {
    let id = egui::Id::new(("device-apps", target.endpoint_prefix(), expected));
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    let load;
    let mut launch = None;
    let mut receipt = None;
    let mut open = None;
    let mut icon_batch = None;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.restored && !state.busy {
            drop(state);
            jobs::restore(ui.ctx(), &shared, target.clone());
            return;
        }
        load = ui
            .add_enabled(
                !state.busy,
                egui::Button::new("Programmkatalog abfragen / aktualisieren"),
            )
            .clicked();
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    state
                        .pending
                        .as_deref()
                        .unwrap_or("Programmanfrage läuft …"),
                );
            });
        }
        if let Some(error) = &state.error {
            let text = super::device_page_layout::explain_remote_error(error);
            ui.colored_label(egui::Color32::RED, text);
        }
        if let Some(saved) = &state.saved {
            ui.separator();
            ui.label(format!("Letzter gespeicherter Auftrag: {}", saved.name));
            ui.label(format!("Request-ID: {}", saved.request.request_id));
            if let Some(result) = &state.receipt {
                receipt_ui(ui, result);
                if let Some(window) = confirmed(target, &state) {
                    if OPENER.get().is_some() {
                        if ui
                            .add_enabled(
                                !state.busy,
                                egui::Button::new("Bestätigtes Appfenster öffnen"),
                            )
                            .clicked()
                        {
                            open = Some(window);
                        }
                    } else {
                        ui.label("Fensterbindung bestätigt; der Anschluss an die aktuelle Fernfensterauswahl ist noch nicht verfügbar.");
                    }
                }
            } else {
                ui.label("Startstatus noch nicht bestätigt. Derselbe Auftrag wird ausschließlich abgefragt.");
            }
            if ui
                .add_enabled(
                    !state.busy,
                    egui::Button::new("Ergebnis derselben Request-ID abfragen"),
                )
                .clicked()
            {
                receipt = Some(saved.request.clone());
            }
        }
        if let Some(catalog) = state.catalog.clone() {
            ui.separator();
            ui.label(format!(
                "Benutzer: {} · Sitzung: {}",
                catalog.session.user_id, catalog.session.session_id
            ));
            for limit in &catalog.limits {
                ui.small(limit);
            }
            if !catalog.session.interactive || !catalog.session.unlocked {
                ui.label("Die aktuelle Hostsitzung ist nicht interaktiv oder gesperrt; ein Start kann Zustimmung verlangen oder verweigert werden.");
            }
            if ui.text_edit_singleline(&mut state.search).changed() {
                state.page = 0;
            }
            let needle = state.search.to_lowercase();
            let apps: Vec<_> = catalog
                .apps
                .iter()
                .filter(|app| {
                    app.name.to_lowercase().contains(&needle)
                        || app.app_id.as_str().to_lowercase().contains(&needle)
                })
                .cloned()
                .collect();
            const PAGE: usize = 12;
            let pages = apps.len().div_ceil(PAGE).max(1);
            state.page = state.page.min(pages - 1);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(state.page > 0, egui::Button::new("Zurück"))
                    .clicked()
                {
                    state.page -= 1;
                }
                ui.label(format!(
                    "Seite {} von {pages} · {} Programme",
                    state.page + 1,
                    apps.len()
                ));
                if ui
                    .add_enabled(state.page + 1 < pages, egui::Button::new("Weiter"))
                    .clicked()
                {
                    state.page += 1;
                }
            });
            let visible: Vec<_> = apps
                .into_iter()
                .skip(state.page * PAGE)
                .take(PAGE)
                .collect();
            // At most the current page's textures; an 8192-entry catalog never
            // allocates thousands of image buffers or parallel requests.
            state
                .icons
                .retain(|app, _| visible.iter().any(|v| v.app_id == *app));
            let unresolved = state.saved.is_some()
                && state.receipt.as_ref().is_none_or(|r| {
                    matches!(
                        r.state,
                        LaunchState::OutcomeUnknown | LaunchState::ConsentRequired
                    )
                });
            if unresolved {
                ui.label("Vor einem weiteren Start zuerst den gespeicherten Beleg abfragen. Lokale Zustimmung erfolgt am Zielgerät.");
                ui.checkbox(&mut state.new_request_ack,"Ich möchte ausdrücklich einen neuen Auftrag erstellen. Der vorige Start kann bereits erfolgt sein.");
            }
            for app in &visible {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        match state.icons.get(&app.app_id) {
                            Some(Icon::Ready(texture)) => {
                                ui.image((texture.id(), egui::vec2(32.0, 32.0)));
                            }
                            Some(Icon::Loading) => {
                                ui.spinner();
                            }
                            Some(Icon::Unavailable(reason)) => {
                                ui.label("◇")
                                    .on_hover_text(format!("Appsymbol nicht verfügbar: {reason}"));
                            }
                            None => {
                                ui.label("◇")
                                    .on_hover_text("Kein bestätigtes Appsymbol im Katalog.");
                            }
                        }
                        ui.label(&app.name);
                        if ui
                            .add_enabled(
                                !state.busy
                                    && (!unresolved || state.new_request_ack)
                                    && app.can_launch,
                                egui::Button::new("Starten"),
                            )
                            .clicked()
                        {
                            launch = Some(app.clone());
                        }
                    });
                    if let Some(reason) = &app.unavailable_reason {
                        ui.small(format!("Start nicht verfügbar: {reason}"));
                    }
                    ui.collapsing("App-ID", |ui| {
                        ui.label(app.app_id.as_str());
                    });
                });
            }
            if visible.is_empty() {
                ui.label("Keine für diese Beziehung sichtbaren Programme passen zur Suche.");
            }
            icon_batch = Some((catalog.revision, visible));
        } else if !state.busy {
            ui.label("Noch kein aktueller Programmkatalog für diese Gerätebeziehung geladen.");
        }
    }
    if load {
        jobs::catalog(
            ui.ctx(),
            &shared,
            target.clone(),
            expected.map(str::to_owned),
        );
    }
    if let Some(app) = launch {
        if let Err(error) = jobs::launch(ui.ctx(), &shared, target.clone(), app) {
            shared.lock().unwrap_or_else(PoisonError::into_inner).error = Some(error);
        }
    }
    if let Some(request) = receipt {
        jobs::receipt(ui.ctx(), &shared, target.clone(), request);
    }
    if let Some(window) = open {
        if let Some(opener) = OPENER.get() {
            if let Err(error) = opener(ui.ctx(), window) {
                shared.lock().unwrap_or_else(PoisonError::into_inner).error = Some(error);
            }
        }
    }
    if !load {
        if let Some((revision, apps)) = icon_batch {
            jobs::icons(ui.ctx(), &shared, target.clone(), revision, apps);
        }
    }
}
fn receipt_ui(ui: &mut egui::Ui, receipt: &LaunchReceipt) {
    let label = match receipt.state {
        LaunchState::OutcomeUnknown => "Startergebnis unklar; Beleg derselben ID erneut abfragen",
        LaunchState::StartedWindowPending => "Gestartet; Fenster noch nicht verfügbar",
        LaunchState::Started => "Gestartet",
        LaunchState::Activated => "Aktiviert",
        LaunchState::ConsentRequired => "Lokale Zustimmung am Zielgerät erforderlich",
        LaunchState::Denied => "Start verweigert",
        LaunchState::Unsupported => "Start auf dem Zielgerät nicht unterstützt",
    };
    ui.label(label);
    ui.label(&receipt.reason);
    if receipt.window.is_none()
        && matches!(
            receipt.state,
            LaunchState::Started | LaunchState::StartedWindowPending | LaunchState::Activated
        )
    {
        ui.small("Noch keine bestätigte Bindung an ein Fernfenster vorhanden. Der Startbeleg enthält keinen auswählbaren Fensterbeleg.");
    }
}
fn confirmed(target: &PeerOpenTarget, state: &State) -> Option<ConfirmedAppWindow> {
    let receipt = state.receipt.as_ref()?;
    let window = receipt.window.as_ref()?;
    if !matches!(
        receipt.state,
        LaunchState::Started | LaunchState::StartedWindowPending | LaunchState::Activated
    ) || window.app_id != receipt.request.app_id
        || window.user_id != receipt.request.user_id
        || window.session_id != receipt.request.session_id
        || window.window_id.is_empty()
        || window.generation == 0
    {
        return None;
    }
    Some(ConfirmedAppWindow {
        target: target.clone(),
        node_id: state.node.clone()?,
        request: receipt.request.clone(),
        window: window.clone(),
    })
}

#[cfg(test)]
#[path = "batches_device_navigation_task_tests.rs"]
mod device_navigation_task_tests;
