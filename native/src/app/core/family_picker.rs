//! Device-local P4 choice. Only the exact authenticated list entry is opened.
use super::screen_target::ScreenTarget;
use crate::screen::core::family_protocol::{FamilyListEntry, FamilyListing};
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Default)]
struct State {
    busy: bool,
    pending: Option<String>,
    error: Option<String>,
    list: Option<FamilyListing>,
    control: bool,
}

pub(crate) fn open_entry(target: &ScreenTarget, entry: &FamilyListEntry) -> Result<(), String> {
    crate::screen::core::family_viewer::FamilyViewer::validate_entry(entry)
        .map_err(|e| e.to_string())?;
    let mut target = target.clone();
    target.source = Some(entry.source.clone());
    super::screen_launch::open_view(&target)
}

/// Main places this panel on a concrete device page, alongside the existing
/// screen/fullscreen actions. Refresh is explicit because a listing pins leases.
pub(crate) fn panel(ui: &mut egui::Ui, target: &ScreenTarget) {
    let id = egui::Id::new((
        "family-picker",
        target.relation,
        &target.relation_id,
        &target.device_id,
        &target.expected_node,
    ));
    let state: Arc<Mutex<State>> = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || {
            Arc::new(Mutex::new(State {
                control: target.control,
                ..Default::default()
            }))
        })
        .clone()
    });
    let mut start = false;
    let mut selected = None;
    {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        if ui
            .add_enabled(
                !state.busy,
                egui::Button::new("Fensterliste abrufen / aktualisieren"),
            )
            .clicked()
        {
            start = true;
        }
        ui.checkbox(&mut state.control, "Fenster mit Steuerrecht öffnen");
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    state
                        .pending
                        .as_deref()
                        .unwrap_or("Fensterliste wird abgerufen …"),
                );
            });
        }
        if let Some(error) = &state.error {
            let text = super::device_page_layout::explain_remote_error(error);
            ui.colored_label(egui::Color32::RED, text);
        }
        if let Some(list) = &state.list {
            if list.entries.is_empty() {
                ui.label("Der Host stellt aktuell keine bestätigten Fensterfamilien bereit.");
            }
            for entry in &list.entries {
                ui.horizontal(|ui| {
                    ui.label(format!("{} · {}", entry.window.app, entry.window.title));
                    if ui.button("Öffnen").clicked() {
                        selected = Some((entry.clone(), state.control));
                    }
                });
            }
            for limitation in &list.limitations {
                ui.label(limitation);
            }
        }
    }
    if let Some((entry, control)) = selected {
        let mut target = target.clone();
        target.control = control;
        if let Err(error) = open_entry(&target, &entry) {
            state.lock().unwrap_or_else(PoisonError::into_inner).error = Some(error);
        }
    }
    if start {
        let target = target.clone();
        let ctx = ui.ctx().clone();
        {
            let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
            s.busy = true;
            s.error = None;
            s.list = None;
        }
        let failed = state.clone();
        let spawned = std::thread::Builder::new()
            .name("family-list".into())
            .spawn(move || {
                let result = crate::daemon::share_worker_snapshot()
                    .and_then(|snapshot| target.check_profiles(&snapshot.profiles))
                    .and_then(|()| target.to_peer())
                    .and_then(|peer| {
                        crate::daemon::screen_families(peer, None, |capability| {
                            state.lock().unwrap_or_else(PoisonError::into_inner).pending =
                                Some(format!("Freigabe für {capability:?} wird erwartet."));
                            ctx.request_repaint();
                        })
                    });
                let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
                s.busy = false;
                s.pending = None;
                match result {
                    Ok(list) => s.list = Some(list),
                    Err(error) => s.error = Some(error),
                }
                ctx.request_repaint();
            });
        if let Err(error) = spawned {
            let mut s = failed.lock().unwrap_or_else(PoisonError::into_inner);
            s.busy = false;
            s.error = Some(error.to_string());
        }
    }
}

/// Registered with share_apps_ui::install_window_opener. Selection is correlated
/// with request ID plus the exact receipt source/generation, never PID/title.
pub(crate) fn open_confirmed(
    ctx: &egui::Context,
    confirmed: super::share_apps_ui::ConfirmedAppWindow,
) -> Result<(), String> {
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("family-launch-open".into())
        .spawn(move || {
            if let Err(error) = resolve_confirmed(confirmed) {
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("family-open-error"), error));
                ctx.request_repaint();
            }
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}
pub(crate) fn notifications(ctx: &egui::Context) {
    let id = egui::Id::new("family-open-error");
    if let Some(error) = ctx.data(|d| d.get_temp::<String>(id)) {
        let mut open = true;
        egui::Window::new("Programmfenster konnte nicht geöffnet werden")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(error);
            });
        if !open {
            ctx.data_mut(|d| d.remove::<String>(id));
        }
    }
}
fn resolve_confirmed(confirmed: super::share_apps_ui::ConfirmedAppWindow) -> Result<(), String> {
    if confirmed.window.app_id != confirmed.request.app_id
        || confirmed.window.user_id != confirmed.request.user_id
        || confirmed.window.session_id != confirmed.request.session_id
    {
        return Err("Fensterbeleg und bestätigter Appstart stimmen nicht überein.".into());
    }
    let listing = crate::daemon::screen_families_for_node(
        confirmed.target.clone(),
        &confirmed.node_id,
        None,
        |_| {},
    )?;
    let entries:Vec<_>=listing.entries.iter().filter(|entry|
        entry.launch_request.as_deref()==Some(confirmed.request.request_id.as_str())
        &&matches!(&entry.source,crate::screen::core::window_list::SourceId::Window(id) if id==&confirmed.window.window_id)
        &&entry.identity.generation==confirmed.window.generation).collect();
    if entries.len() != 1 {
        return Err(
            "Der Host bestätigt kein eindeutiges aktuelles Fenster für diesen Startauftrag.".into(),
        );
    }
    let mut target = match confirmed.target {
        crate::share::PeerOpenTarget::Direct { contact_id } => ScreenTarget::direct(
            &contact_id,
            Some(confirmed.request.target_device_id.as_str()),
        ),
        crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
            ScreenTarget::room(&room_id, &device_id)
        }
        _ => return Err("Der Appstart nennt kein konkretes bestätigtes Gerät.".into()),
    };
    target.expected_node = Some(confirmed.node_id);
    target.control = true;
    open_entry(&target, entries[0])
}
