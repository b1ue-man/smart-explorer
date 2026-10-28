//! Device-scoped programs (design 6, user rule r3 „der rest muss laufen“):
//! the catalog loads by itself when the device is present, an unclear start
//! is clarified by se with the same receipt whenever the host comes back or
//! its windows change, a start that waits for a locked host session starts
//! again by itself once the session is unlocked, and a confirmed window
//! opens by itself. Independent from Exec; the double-start guard stays.
use super::share_rdp_ui::feed::{self, FeedRead};
use crate::desktop::core::tiles;
use crate::share::peer_select::present_relation;
use crate::{
    apps::{AppId, CatalogSnapshot, LaunchReceipt, LaunchRequest, LaunchState, WindowBinding},
    share::{PeerOpenTarget, ShareProfiles},
};
use eframe::egui;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};
#[path = "share_apps_catalog.rs"]
mod catalog;
#[path = "share_apps_jobs.rs"]
mod jobs;
#[path = "share_apps_store.rs"]
mod store;

/// Shown while the outcome of a start is being clarified (design 6 F4).
pub(crate) const CLARIFYING: &str = "Startergebnis wird geklärt";
/// A start waits for the locked host session (design 6 F5).
pub(crate) const LOCKED: &str = "Hostsitzung gesperrt – zum Entsperren den Bildschirm öffnen";

enum Icon {
    Loading,
    Ready(egui::TextureHandle),
    Unavailable(String),
}
/// What the device feed says about the host now; every change is an event
/// on which se clarifies an open start (design 6 F4, F5). No timer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostEvent {
    pub(crate) present: bool,
    pub(crate) windows: Vec<String>,
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
    /// The last host state seen and the number of its changes.
    event: Option<HostEvent>,
    events: u64,
    /// The event at which the catalog / the receipt was last asked.
    catalog_round: Option<u64>,
    receipt_round: Option<u64>,
    /// A start that waits for the unlocked host session.
    relaunch: Option<AppId>,
    /// The request whose confirmed window was already opened.
    opened: Option<LaunchRequest>,
}
type Shared = Arc<Mutex<State>>;
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
    match present_relation(profiles, device, key) {
        Some(target) => panel(ui, &target, Some(device)),
        None => {
            ui.label(super::share_rdp_ui::OFFLINE);
        }
    }
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

/// The host state of `target` in the background service's feed.
fn host_event(ctx: &egui::Context, target: &PeerOpenTarget) -> HostEvent {
    let read = feed::read(ctx, |snapshot| {
        let Some((device, key)) = tiles::identity_of(&snapshot.profiles, target) else {
            return HostEvent::default();
        };
        HostEvent {
            present: present_relation(&snapshot.profiles, &device, &key).is_some(),
            windows: tiles::feed_of(snapshot, &device, &key)
                .map(|feed| feed.windows.into_iter().map(|w| w.source).collect())
                .unwrap_or_default(),
        }
    });
    match read {
        FeedRead::Ready(event) => event,
        FeedRead::Pending | FeedRead::Unavailable(_) => HostEvent::default(),
    }
}

/// A receipt that is not final yet: se keeps asking for the same request
/// on every host event (design 6 F4).
pub(crate) fn unresolved(saved: bool, receipt: Option<&LaunchReceipt>) -> bool {
    saved
        && receipt.is_none_or(|r| {
            matches!(
                r.state,
                LaunchState::OutcomeUnknown | LaunchState::StartedWindowPending
            )
        })
}

/// What one frame asks the host without a click (design 6): the catalog
/// once the device is present, the same receipt on every host event while
/// it is not final, and the catalog again for a start that waits for an
/// unlocked session.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AutoWork {
    pub(crate) catalog: bool,
    pub(crate) receipt: bool,
}
pub(crate) fn auto_work(
    event: &HostEvent,
    events: u64,
    has_catalog: bool,
    catalog_round: Option<u64>,
    receipt_open: bool,
    receipt_round: Option<u64>,
    relaunch: bool,
) -> AutoWork {
    if !event.present {
        return AutoWork::default();
    }
    AutoWork {
        catalog: (!has_catalog || relaunch) && catalog_round != Some(events),
        receipt: receipt_open && receipt_round != Some(events),
    }
}

fn panel(ui: &mut egui::Ui, target: &PeerOpenTarget, expected: Option<&str>) {
    let id = egui::Id::new(("device-apps", target.endpoint_prefix(), expected));
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    let event = host_event(ui.ctx(), target);
    let mut load;
    let mut launch = None;
    let mut receipt = None;
    let mut open = None;
    let mut icon_batch = None;
    let mut screen = false;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.restored && !state.busy {
            drop(state);
            jobs::restore(ui.ctx(), &shared, target.clone());
            return;
        }
        if state.event.as_ref() != Some(&event) {
            state.event = Some(event.clone());
            state.events += 1;
        }
        let open_receipt = unresolved(state.saved.is_some(), state.receipt.as_ref());
        let work = auto_work(
            &event,
            state.events,
            state.catalog.is_some(),
            state.catalog_round,
            open_receipt,
            state.receipt_round,
            state.relaunch.is_some(),
        );
        load = !state.busy && work.catalog;
        if !state.busy && work.receipt && !load {
            state.receipt_round = Some(state.events);
            receipt = state.saved.as_ref().map(|saved| saved.request.clone());
        }
        if load {
            state.catalog_round = Some(state.events);
        }
        load |= ui
            .add_enabled(!state.busy, egui::Button::new("Neu laden"))
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
        if !event.present && state.catalog.is_none() {
            ui.label(super::share_rdp_ui::OFFLINE);
        }
        if let Some(error) = &state.error {
            let text = super::device_page_layout::explain_remote_error(error);
            ui.colored_label(egui::Color32::RED, text);
        }
        if let Some(saved) = &state.saved {
            ui.separator();
            ui.label(format!("Letzter Start: {}", saved.name));
            match &state.receipt {
                Some(result) => {
                    let locked = receipt_ui(ui, result, state.catalog.as_ref());
                    if locked && ui.button("Bildschirm steuern").clicked() {
                        screen = true;
                    }
                }
                None => {
                    ui.label(CLARIFYING);
                }
            }
            if let Some(window) = confirmed(target, &state) {
                if OPENER.get().is_some() {
                    if state.opened.as_ref() == Some(&window.request) {
                        ui.label("Programmfenster wurde geöffnet.");
                    } else {
                        state.opened = Some(window.request.clone());
                        open = Some(window);
                    }
                } else {
                    ui.label("Fensterbindung bestätigt; der Anschluss an die aktuelle Fernfensterauswahl ist noch nicht verfügbar.");
                }
            }
        }
        // A start that waited for the locked session runs again by itself
        // with a new request once the catalog reports it unlocked (F5).
        if let Some(app) = state.relaunch.clone() {
            let waiting = state
                .receipt
                .as_ref()
                .is_some_and(|r| r.state == LaunchState::ConsentRequired);
            let unlocked = state.catalog.as_ref().map(|c| c.session.unlocked);
            if !waiting {
                state.relaunch = None;
            } else if unlocked == Some(true) && !state.busy && !load {
                launch = state
                    .catalog
                    .as_ref()
                    .and_then(|c| c.apps.iter().find(|a| a.app_id == app).cloned());
                state.relaunch = None;
            }
        }
        if let Some(catalog) = state.catalog.clone() {
            icon_batch = catalog::show(ui, &mut state, &catalog, open_receipt, &mut launch);
        } else if !state.busy && event.present {
            ui.label("Der Programmkatalog wird geladen, sobald das Gerät antwortet.");
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
    if screen {
        let device = expected.map(str::to_string).or_else(|| {
            let state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let saved = state.saved.as_ref()?;
            Some(saved.request.target_device_id.as_str().to_string())
        });
        if let Err(error) = open_screen(target, device.as_deref()) {
            shared.lock().unwrap_or_else(PoisonError::into_inner).error = Some(error);
        }
    }
    if !load {
        if let Some((revision, apps)) = icon_batch {
            jobs::icons(ui.ctx(), &shared, target.clone(), revision, apps);
        }
    }
}

/// „Bildschirm steuern“ beside a start that waits for the locked session.
fn open_screen(target: &PeerOpenTarget, device: Option<&str>) -> Result<(), String> {
    let mut screen = match target {
        PeerOpenTarget::Direct { contact_id } => {
            super::screen_target::ScreenTarget::direct(contact_id, device)
        }
        PeerOpenTarget::RoomDevice { room_id, device_id } => {
            super::screen_target::ScreenTarget::room(room_id, device_id)
        }
    };
    screen.control = true;
    super::screen_launch::open_view(&screen)
}

/// The receipt in words; `true` when the start waits for the locked host
/// session (a desktop host asks for consent only then, design 6 F5).
fn receipt_ui(
    ui: &mut egui::Ui,
    receipt: &LaunchReceipt,
    catalog: Option<&CatalogSnapshot>,
) -> bool {
    let locked = receipt.state == LaunchState::ConsentRequired
        && catalog.is_none_or(|catalog| !catalog.session.unlocked);
    let label = match receipt.state {
        LaunchState::OutcomeUnknown => CLARIFYING,
        LaunchState::StartedWindowPending => "Gestartet; Fenster noch nicht verfügbar",
        LaunchState::Started => "Gestartet",
        LaunchState::Activated => "Aktiviert",
        LaunchState::ConsentRequired if locked => LOCKED,
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
    locked
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
