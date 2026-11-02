//! Entry points of the compact rights flow (plan 2026-10-31 §F, package G2):
//! the row of preset buttons that every place uses (F4), the cards on the
//! device and room pages that show open rights and reopen the window, and the
//! rights chosen in the same step as a Direct code, a PIN offer, an
//! automatic acceptance or a new room (F5, F6).
use eframe::egui::{self, RichText};

use super::share_rights_core::CoreRights;
use super::share_rights_model::{
    code_rights_text, origin_text, RightsDraft, RightsSubject, RightsWindowState,
};
use crate::app::{device_page_layout as layout, theme, App};
use crate::share::connection_rights::{ConnectionRights, RightsPreset, RightsTarget};
use crate::share::connection_rights_actions::RightsChange;

pub(in crate::app) fn state_id() -> egui::Id {
    egui::Id::new("share_rights_window_state")
}

/// Runs `edit` on the window's memory (kept in egui memory, not saved).
pub(in crate::app) fn with_state<R>(
    ctx: &egui::Context,
    edit: impl FnOnce(&mut RightsWindowState) -> R,
) -> R {
    let mut state = ctx
        .data_mut(|data| data.get_temp::<RightsWindowState>(state_id()))
        .unwrap_or_default();
    let result = edit(&mut state);
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
    result
}

/// The one row of preset buttons (F4). `ask` adds a first button that
/// stands for „no rights chosen in advance“ (the device waits and the
/// window opens when it connects). Returns the clicked button: `Some(None)`
/// is the `ask` button, `Some(Some(preset))` a preset.
pub(in crate::app) fn preset_buttons(
    ui: &mut egui::Ui,
    selected: Option<RightsPreset>,
    ask: Option<(&str, bool)>,
) -> Option<Option<RightsPreset>> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        if let Some((label, on)) = ask {
            if ui
                .selectable_label(on, label)
                .on_hover_text("Das Gerät verbindet ohne Rechte; das Fenster „Rechte“ öffnet sich, sobald es verbunden ist.")
                .clicked()
            {
                clicked = Some(None);
            }
        }
        let ask_on = ask.is_some_and(|(_, on)| on);
        for preset in RightsPreset::ALL {
            let on = !ask_on && selected == Some(preset);
            if ui
                .selectable_label(on, preset.label())
                .on_hover_text(preset.description())
                .clicked()
            {
                clicked = Some(Some(preset));
            }
        }
    });
    clicked
}

fn home() -> Option<String> {
    CoreRights::new().home
}

/// Commits what a core call returned: the window's copy of the profiles,
/// the background worker (which applies stored rights after they are
/// persisted) and a notice.
pub(in crate::app) fn apply_change(app: &mut App, change: RightsChange, notice: &str) {
    app.share_profiles = change.profiles;
    app.share_next_poll_at = std::time::Instant::now();
    if let Err(error) = crate::daemon::refresh_share_worker_checked() {
        app.error_msg = Some(format!(
            "Rechte gespeichert, Worker nicht neu geladen: {error}"
        ));
    }
    app.notice = Some((notice.to_string(), std::time::Instant::now()));
}

/// Opens the window for `draft` (device page, Direct code).
pub(in crate::app) fn open_draft(ctx: &egui::Context, draft: RightsDraft) {
    with_state(ctx, |state| state.open(draft));
    ctx.request_repaint();
}

/// Opens the window for a device or room that exists: the open
/// confirmation when it waits, else its current rights. `false` when the
/// target is gone.
pub(in crate::app) fn open_for_target(
    app: &App,
    ctx: &egui::Context,
    target: &RightsTarget,
) -> bool {
    let profiles = &app.share_profiles;
    if let Some(view) = profiles
        .pending_rights_views()
        .into_iter()
        .find(|view| &view.target == target)
    {
        open_draft(ctx, RightsDraft::for_pending(&view));
        return true;
    }
    let found = match target {
        RightsTarget::Direct {
            device_id,
            public_key,
        } => profiles
            .grant_for(device_id)
            .map(|grant| grant.device_name.clone())
            .zip(profiles.current_direct_rights(device_id, public_key)),
        RightsTarget::Room { profile_id } => profiles
            .rooms
            .iter()
            .find(|room| &room.id == profile_id)
            .map(|room| room.name.clone())
            .zip(profiles.current_room_rights(profile_id)),
    };
    match found {
        Some((name, rights)) => {
            let subject = RightsSubject::Connection(target.clone());
            open_draft(ctx, RightsDraft::new(subject, &name, rights));
            true
        }
        None => false,
    }
}

/// Whether the Direct grant of `device_id` waits for its rights.
pub(in crate::app) fn direct_open(profiles: &crate::share::ShareProfiles, device_id: &str) -> bool {
    profiles
        .pending_rights
        .iter()
        .any(|pending| pending.target.is_direct(device_id))
}

/// Whether the room `profile_id` waits for its rights.
pub(in crate::app) fn room_open(profiles: &crate::share::ShareProfiles, profile_id: &str) -> bool {
    profiles
        .pending_rights
        .iter()
        .any(|pending| pending.target.is_room(profile_id))
}

/// The right-hand line of a device in the list: „Rechte offen“ while the
/// connection waits, else „Darf schreiben · Befehle: erlaubt · …“.
pub(in crate::app) fn list_line(
    app: &App,
    ui: &mut egui::Ui,
    grant: &crate::share::DirectGrant,
) -> String {
    if direct_open(&app.share_profiles, &grant.device_id) {
        return "Rechte offen · noch nichts bestätigt".to_string();
    }
    let mut parts = vec![if grant.write {
        "Darf schreiben".to_string()
    } else {
        "Nur lesen".to_string()
    }];
    let key = crate::app::share_exec_ui::direct_target_key(&grant.device_id, &grant.fingerprint);
    if let Some(access) = crate::app::share_exec_ui::exec_summary(app, ui, &key) {
        parts.push(access);
    }
    parts.join(" · ")
}

/// „Rechte festlegen …“ in a device row while its rights are open; returns
/// the target when it was clicked.
pub(in crate::app) fn open_button(
    ui: &mut egui::Ui,
    profiles: &crate::share::ShareProfiles,
    grant: &crate::share::DirectGrant,
) -> Option<RightsTarget> {
    if !direct_open(profiles, &grant.device_id) {
        return None;
    }
    ui.button("Rechte festlegen …")
        .clicked()
        .then(|| RightsTarget::Direct {
            device_id: grant.device_id.clone(),
            public_key: grant.public_key.clone(),
        })
}

/// „Rechte“ card of a device page: the rights now, „Rechte offen“ when it
/// still waits, and the button that opens the window.
pub(in crate::app) fn device_card(
    app: &mut App,
    ui: &mut egui::Ui,
    device_id: &str,
    public_key: &str,
) {
    let target = RightsTarget::Direct {
        device_id: device_id.to_string(),
        public_key: public_key.to_string(),
    };
    let Some(rights) = app
        .share_profiles
        .current_direct_rights(device_id, public_key)
    else {
        return;
    };
    let pending = app.share_profiles.pending_rights_of(&target).cloned();
    rights_card(
        app,
        ui,
        &target,
        &rights,
        pending.map(|p| p.origin),
        "Dateien, Schreiben, Befehle und Desktop",
    );
}

/// „Rechte“ card of a room page.
pub(in crate::app) fn room_card(app: &mut App, ui: &mut egui::Ui, profile_id: &str) {
    let target = RightsTarget::Room {
        profile_id: profile_id.to_string(),
    };
    let Some(rights) = app.share_profiles.current_room_rights(profile_id) else {
        return;
    };
    let pending = app.share_profiles.pending_rights_of(&target).cloned();
    rights_card(
        app,
        ui,
        &target,
        &rights,
        pending.map(|p| p.origin),
        "Was Mitglieder dieses Raums dürfen",
    );
}

fn rights_card(
    app: &mut App,
    ui: &mut egui::Ui,
    target: &RightsTarget,
    rights: &ConnectionRights,
    waiting: Option<crate::share::connection_rights::RightsOrigin>,
    hint: &str,
) {
    let mut open = false;
    layout::card(ui, "Rechte", Some(hint), |ui| {
        match waiting {
            Some(origin) => {
                ui.colored_label(
                    theme::warning(ui),
                    format!(
                        "Rechte offen · {}. Bis du sie bestätigst, darf hier nichts geschehen.",
                        origin_text(origin)
                    ),
                );
            }
            None => {
                ui.label(rights.summary());
            }
        }
        let label = if waiting.is_some() {
            "Rechte festlegen …"
        } else {
            "Rechte einstellen …"
        };
        open = ui.button(label).clicked();
    });
    if open {
        open_for_target(app, ui.ctx(), target);
    }
}

/// The Direct code's rights in the same place as the code (F5): what it
/// gives now, the preset buttons (stored at once) and „Anpassen …“.
pub(in crate::app) fn code_row(app: &mut App, ui: &mut egui::Ui) {
    let current = app.share_profiles.code_rights.clone();
    ui.label(RichText::new(code_rights_text(current.as_ref())).color(theme::muted(ui)));
    let selected = current.as_ref().and_then(ConnectionRights::preset);
    let ask_on = current.is_none();
    let clicked = preset_buttons(ui, selected, Some(("Beim Verbinden erfragen", ask_on)));
    let mut custom = false;
    ui.horizontal(|ui| {
        custom = ui.button("Anpassen …").clicked();
    });
    if let Some(choice) = clicked {
        store_code_rights(app, choice.map(RightsPreset::rights));
    }
    if custom {
        open_draft(ui.ctx(), RightsDraft::for_code(current));
    }
}

/// Stores the Direct code's rights (`None`: devices wait for them).
pub(in crate::app) fn store_code_rights(app: &mut App, rights: Option<ConnectionRights>) {
    let result = crate::share::connection_rights_actions::set_code_rights(home(), rights);
    match result {
        Ok(change) => apply_change(app, change, "Rechte des Direkt-Codes gespeichert"),
        Err(error) => app.error_msg = Some(format!("Rechte des Direkt-Codes: {error}")),
    }
}

/// The preset a new PIN offer hands to the device that pairs with it (F5):
/// the buttons while no offer is visible, the rights the running offer
/// really carries (as the worker reports them, also after the window was
/// restarted) as text while one is. `None`: no rights in advance (the
/// device waits). Only Direct offers carry rights; a room's rights are set
/// with the room.
pub(in crate::app) fn offer_choice(
    ui: &mut egui::Ui,
    direct: bool,
    active: Option<&crate::share::discovery_state::ActiveDiscoveryOffer>,
) -> Option<ConnectionRights> {
    if !direct {
        return None;
    }
    if let Some(offer) = active {
        ui.label(
            RichText::new(offer.rights_sentence())
                .small()
                .color(theme::muted(ui)),
        );
        return None;
    }
    let id = egui::Id::new("share_offer_rights_preset");
    let stored = ui
        .data_mut(|data| data.get_temp::<Option<RightsPreset>>(id))
        .unwrap_or(None);
    let text = match stored {
        Some(preset) => format!(
            "Das Gerät, das mit dieser PIN verbindet, erhält: {}.",
            preset.label()
        ),
        None => {
            "Das Gerät, das mit dieser PIN verbindet, wartet ohne Rechte; du legst sie danach fest."
                .to_string()
        }
    };
    ui.label(
        RichText::new("Rechte für das Gerät, das verbindet")
            .small()
            .color(theme::muted(ui)),
    );
    if let Some(choice) = preset_buttons(ui, stored, Some(("Danach festlegen", stored.is_none()))) {
        ui.data_mut(|data| data.insert_temp(id, choice));
        return choice.map(RightsPreset::rights);
    }
    ui.label(RichText::new(text).small().color(theme::muted(ui)));
    stored.map(RightsPreset::rights)
}

/// The preset for a room that is about to be created (F5); `None` means the
/// window „Rechte für <Raum>“ opens right after creation.
pub(in crate::app) fn room_create_choice(ui: &mut egui::Ui) -> Option<RightsPreset> {
    let id = egui::Id::new("share_room_create_rights_preset");
    let mut stored = ui
        .data_mut(|data| data.get_temp::<Option<RightsPreset>>(id))
        .unwrap_or(None);
    ui.label(
        RichText::new("Rechte der Mitglieder")
            .small()
            .color(theme::muted(ui)),
    );
    if let Some(choice) = preset_buttons(ui, stored, Some(("Danach festlegen", stored.is_none()))) {
        stored = choice;
        ui.data_mut(|data| data.insert_temp(id, stored));
    }
    stored
}

/// Forgets the room preset after a room was created, so the next one asks
/// again from the neutral choice.
pub(in crate::app) fn reset_room_create_choice(ui: &egui::Ui) {
    ui.data_mut(|data| {
        data.insert_temp::<Option<RightsPreset>>(
            egui::Id::new("share_room_create_rights_preset"),
            None,
        )
    });
}

/// Finishes a room just created here: the preset chosen in the same step is
/// its rights, else the room waits for them (the window opens).
pub(in crate::app) fn finish_room(
    app: &mut App,
    ctx: &egui::Context,
    profile_id: &str,
    preset: Option<RightsPreset>,
) {
    let rights = preset.map(RightsPreset::rights);
    let result = crate::share::connection_rights_actions::finish_room_creation(
        home(),
        profile_id,
        rights.as_ref(),
    );
    match result {
        Ok(change) => {
            let notice = match preset {
                Some(preset) => format!("Raum erstellt: {}", preset.label()),
                None => "Raum erstellt: Rechte noch festlegen".to_string(),
            };
            apply_change(app, change, &notice);
            if preset.is_some() {
                // The rights are set: an older worker snapshot that still
                // lists the room as joined must not open the window.
                let target = RightsTarget::Room {
                    profile_id: profile_id.to_string(),
                };
                with_state(ctx, |state| {
                    state.settled.push((target, std::time::Instant::now()));
                });
            }
        }
        Err(error) => app.error_msg = Some(format!("Raum-Rechte: {error}")),
    }
}
