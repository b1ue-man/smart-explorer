//! The compact rights window „Rechte für <Gerät/Raum>“ (plan 2026-10-31 §F,
//! F1, F3, F4; package G2). It opens by itself for every connection that
//! waits for its rights (accepted or auto-accepted request, PIN pairing,
//! created or joined room, share-back), on the side that shares: presets in
//! one row on top, one row per right, „Mehr“ for the rest, „Bestätigen“ and
//! „Abbrechen“. Nothing is stored before „Bestätigen“; „Abbrechen“ keeps the
//! connection without rights. The close button puts the window off: the
//! confirmation stays open on the device page and in the device list.
use eframe::egui::{self, RichText};

use super::share_rights_choice_ui::{apply_change, preset_buttons, state_id};
use super::share_rights_core::CoreRights;
use super::share_rights_model::{origin_text, RightsDraft, RightsSubject, RightsWindowState};
use super::share_rights_more_ui::{self as more, MoreAction, MoreInfo};
use super::share_rights_rows_ui::{self as rows, FolderInfo};
use crate::app::{theme, App};
use crate::share::connection_rights::RightsTarget;

/// What the user did in this frame.
enum Outcome {
    Keep,
    Confirm,
    Cancel,
    Later,
}

/// The window, drawn every frame from the status bar (next to the SSH host
/// key window); it draws nothing while no confirmation is open.
pub(in crate::app) fn window(app: &mut App, ctx: &egui::Context) {
    let stored = ctx.data_mut(|data| data.get_temp::<RightsWindowState>(state_id()));
    let pending = app.share_profiles.pending_rights_views();
    if stored.is_none() && pending.is_empty() {
        return;
    }
    let mut state = stored.unwrap_or_default();
    state.sync(&pending, std::time::Instant::now());
    if let Some(mut draft) = state.draft.take() {
        let (outcome, action) = show(app, ctx, &mut draft, state.error.as_deref());
        let now = crate::share::core_now_secs();
        let mut backend = CoreRights::new();
        match outcome {
            Outcome::Keep => state.draft = Some(draft),
            Outcome::Later => {
                state.draft = Some(draft);
                state.later();
            }
            Outcome::Confirm => match draft.confirm(&mut backend, now) {
                Ok(change) => {
                    let summary = draft
                        .finish(now)
                        .map(|rights| rights.summary())
                        .unwrap_or_default();
                    let notice = match draft.subject {
                        RightsSubject::Code => format!("Direkt-Code vergibt: {summary}"),
                        RightsSubject::Connection(_) => {
                            format!("Rechte für {} gesetzt: {summary}", draft.name)
                        }
                    };
                    apply_change(app, change, &notice);
                    state.settled(&draft, std::time::Instant::now());
                }
                Err(error) => {
                    state.error = Some(error);
                    state.draft = Some(draft);
                }
            },
            Outcome::Cancel => match draft.cancel(&mut backend) {
                Ok(change) => {
                    if let Some(change) = change {
                        let notice = format!("{} bleibt verbunden, ohne Rechte", draft.name);
                        apply_change(app, change, &notice);
                    }
                    state.settled(&draft, std::time::Instant::now());
                }
                Err(error) => {
                    state.error = Some(error);
                    state.draft = Some(draft);
                }
            },
        }
        if let Some(action) = action {
            run_action(app, ctx, action);
        }
    }
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
}

/// The folders, accounts and shortcuts the window needs from the profiles.
struct Facts {
    folders: FolderInfo,
    more: MoreInfo,
}

fn facts(app: &App, draft: &RightsDraft) -> Facts {
    let profiles = &app.share_profiles;
    let exports = &profiles.default_direct_exports;
    let device = match &draft.subject {
        RightsSubject::Connection(RightsTarget::Direct {
            device_id,
            public_key,
        }) => Some((device_id.clone(), public_key.clone())),
        _ => None,
    };
    let programs = device
        .as_ref()
        .and_then(|(device_id, _)| profiles.grant_for(device_id))
        .map_or(0, |grant| grant.exec.programs.len());
    let contact = device.as_ref().and_then(|(device_id, key)| {
        profiles
            .direct_contacts
            .iter()
            .find(|contact| {
                contact.remote_device_id.as_deref() == Some(device_id.as_str())
                    && contact.remote_public_key.as_deref() == Some(key.as_str())
            })
            .map(|contact| (contact.id.clone(), contact.relation.share_back))
    });
    Facts {
        folders: FolderInfo {
            exported: exports
                .roots
                .iter()
                .map(|root| (root.label.clone(), root.path.clone()))
                .collect(),
            connections: exports.shares_connections(),
        },
        more: MoreInfo {
            device,
            programs,
            contact,
            other_accounts: crate::share::exec_run_as_supported(),
        },
    }
}

fn show(
    app: &App,
    ctx: &egui::Context,
    draft: &mut RightsDraft,
    error: Option<&str>,
) -> (Outcome, Option<MoreAction>) {
    let facts = facts(app, draft);
    let mut outcome = Outcome::Keep;
    let mut action = None;
    let mut open = true;
    let title = match draft.subject {
        RightsSubject::Code => "Rechte des Direkt-Codes".to_string(),
        RightsSubject::Connection(_) => format!("Rechte für {}", draft.name),
    };
    crate::app::dialog_geometry::window(ctx, "share_rights_ui.0", title)
        .id(egui::Id::new("share_rights_window"))
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .show(ctx, |ui| {
            body(ui, draft, &facts, error, &mut outcome, &mut action);
        });
    if !open {
        outcome = Outcome::Later;
    }
    (outcome, action)
}

fn body(
    ui: &mut egui::Ui,
    draft: &mut RightsDraft,
    facts: &Facts,
    error: Option<&str>,
    outcome: &mut Outcome,
    action: &mut Option<MoreAction>,
) {
    match (draft.origin, &draft.subject) {
        (Some(origin), _) => {
            ui.label(
                RichText::new(format!(
                    "{}. Bis du bestätigst, hat die Verbindung keine Rechte.",
                    origin_text(origin)
                ))
                .color(theme::muted(ui)),
            );
        }
        (None, RightsSubject::Code) => {
            ui.label(
                RichText::new(
                    "Diese Rechte bekommt jedes Gerät, das mit deinem Direkt-Code verbindet.",
                )
                .color(theme::muted(ui)),
            );
        }
        (None, _) => {}
    }
    if let Some(Some(preset)) = preset_buttons(ui, draft.selected_preset(), None) {
        draft.apply_preset(preset);
    }
    ui.separator();
    egui::Grid::new("share_rights_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Ordner");
            rows::folders(ui, draft, &facts.folders);
            ui.end_row();
            ui.label("Schreiben");
            rows::write(ui, draft);
            ui.end_row();
            rows::decisions(ui, draft);
            ui.label("Ablauf");
            rows::expiry(ui, draft);
            ui.end_row();
        });
    egui::CollapsingHeader::new("Mehr")
        .id_salt("share_rights_more")
        .default_open(false)
        .show(ui, |ui| {
            if let Some(clicked) = more::ui(ui, draft, &facts.more) {
                *action = Some(clicked);
            }
        });
    if let Some(error) = error {
        ui.colored_label(theme::danger(ui), error);
    }
    ui.separator();
    ui.horizontal(|ui| {
        let confirm = match draft.subject {
            RightsSubject::Code => "Für den Code speichern",
            RightsSubject::Connection(_) => "Bestätigen",
        };
        if ui.button(confirm).clicked() {
            *outcome = Outcome::Confirm;
        }
        let cancel_hint = match draft.origin {
            Some(_) => "Die Verbindung bleibt bestehen, ohne Rechte. Über die Geräteseite stellst du sie später ein.",
            None => "Schließt das Fenster, ohne etwas zu ändern.",
        };
        if ui.button("Abbrechen").on_hover_text(cancel_hint).clicked() {
            *outcome = Outcome::Cancel;
        }
        if draft.origin.is_some()
            && ui
                .small_button("Später")
                .on_hover_text("Rechte bleiben offen; „Rechte offen“ steht an der Geräteseite.")
                .clicked()
        {
            *outcome = Outcome::Later;
        }
    });
}

/// A shortcut of „Mehr“ that needs the app. The window stays open; the
/// page opens behind it.
fn run_action(app: &mut App, ctx: &egui::Context, action: MoreAction) {
    match action {
        MoreAction::DevicePage(device_id, public_key) => {
            app.open_share_device_page(&device_id, &public_key);
        }
        MoreAction::Exports => app.open_share_exports_page(),
        MoreAction::ShareBack(contact_id, on) => {
            let home = CoreRights::new().home;
            match crate::share::set_direct_share_back(home, &contact_id, on) {
                Ok(change) => apply_change(
                    app,
                    crate::share::connection_rights_actions::RightsChange {
                        profiles: change.profiles,
                        changed: true,
                    },
                    if on {
                        "Rückfreigabe an"
                    } else {
                        "Rückfreigabe aus"
                    },
                ),
                Err(error) => app.error_msg = Some(format!("Rückfreigabe: {error}")),
            }
        }
    }
    ctx.request_repaint();
}
