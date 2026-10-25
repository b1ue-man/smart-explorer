//! „Zugriff für alle Mitglieder…“ of a room (PRM-M9, C1 E-a, D.3 T7): one
//! dialog with the controls of the access dialog; its one confirmation („Für
//! n Mitglieder erlauben“) writes the same right for every member admitted
//! when the dialog opened, each at its own revision. Each member gets a right
//! of its own; members admitted later get nothing (no room-wide rule).
use eframe::egui::{self, RichText};

use super::access_dialog::AccessDraft;
use super::programs_ui::{self, controls};
use super::{cached_provider_status, exec_device_views, ExecDeviceView};
use crate::app::{theme, App};
use crate::share::access_grant::{AccessGrant, AccessGrantChange};
use crate::share::{ExecProviderStatus, RoomProfile};

/// One member of the snapshot the dialog took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotMember {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) revision: u64,
}

/// The open room dialog: the draft and the members it applies to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RoomDialog {
    pub(super) draft: AccessDraft,
    pub(super) members: Vec<SnapshotMember>,
}

/// One change of the confirmation: target key, change, expected revision.
pub(super) type Planned = (String, AccessGrantChange, u64);

/// The members of `room` that are admitted now, from `views`.
pub(super) fn snapshot(views: &[ExecDeviceView], room: &RoomProfile) -> Vec<SnapshotMember> {
    let admitted = |view: &&ExecDeviceView| {
        room.members.iter().any(|member| {
            member.is_admitted()
                && member.device_id == view.device_id
                && member.fingerprint == view.fingerprint
        })
    };
    views
        .iter()
        .filter(|view| view.room_id.as_deref() == Some(room.room_id.as_str()))
        .filter(|view| view.base_authorized)
        .filter(admitted)
        .map(|view| SnapshotMember {
            key: view.target_key.clone(),
            name: view.device_name.clone(),
            revision: view.policy_revision,
        })
        .collect()
}

/// The same change for every member of the snapshot, each at its revision.
pub(super) fn planned(dialog: &RoomDialog, now: i64) -> Vec<Planned> {
    let change = dialog.draft.change(now);
    let each = |member: &SnapshotMember| (member.key.clone(), change.clone(), member.revision);
    dialog.members.iter().map(each).collect()
}

fn dialog_id() -> egui::Id {
    egui::Id::new("share_access_room_dialog")
}

pub(super) fn open(ctx: &egui::Context, dialog: RoomDialog) {
    ctx.data_mut(|data| data.insert_temp(dialog_id(), dialog));
}

fn current(ctx: &egui::Context) -> Option<RoomDialog> {
    ctx.data_mut(|data| data.get_temp::<RoomDialog>(dialog_id()))
}

fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<RoomDialog>(dialog_id()));
}

/// The button of the room page and, while it is open, its dialog.
pub(in crate::app) fn ui_room_access(app: &mut App, ui: &mut egui::Ui, room: &RoomProfile) {
    let views = exec_device_views(&app.share_profiles);
    let members = snapshot(&views, room);
    let ready = room.auto_join && !members.is_empty();
    let button = egui::Button::new("Zugriff für alle Mitglieder…");
    let response = ui.add_enabled(ready, button).on_disabled_hover_text(
        "Erst wenn der Raum beigetreten ist und mindestens ein Mitglied zugelassen wurde.",
    );
    if response.clicked() {
        let accounts = programs_ui::account_options(&views);
        let right = (AccessGrant::default(), Vec::new());
        let draft = AccessDraft::of(&room.id, &room.name, 0, right, accounts);
        open(ui.ctx(), RoomDialog { draft, members });
    }
    let provider = cached_provider_status(ui);
    let now = crate::share::core_now_secs();
    if let Some(planned) = show(ui.ctx(), &provider, &room.id, now) {
        apply_all(app, &views, planned);
    }
}

/// The dialog of the room `room_key` while it is open; the changes of its
/// confirmation.
pub(super) fn show(
    ctx: &egui::Context,
    provider: &ExecProviderStatus,
    room_key: &str,
    now: i64,
) -> Option<Vec<Planned>> {
    let mut dialog = current(ctx).filter(|dialog| dialog.draft.key == room_key)?;
    let (mut visible, mut done, mut result) = (true, false, None);
    let count = dialog.members.len();
    crate::app::dialog_geometry::window(ctx, "share_access_room_ui.0", format!(
        "Zugriff für alle Mitglieder von {}",
        dialog.draft.title
    ))
    .id(dialog_id())
    .collapsible(false)
    .resizable(true)
    .default_width(480.0)
    .open(&mut visible)
    .show(ctx, |ui| {
        let names: Vec<&str> = dialog.members.iter().map(|m| m.name.as_str()).collect();
        ui.label(format!(
            "Gilt für {count} zugelassene Mitglieder: {}.",
            names.join(", ")
        ));
        let scope = "Jedes Mitglied erhält ein eigenes Recht. Später zugelassene Mitglieder \
                         erhalten nichts.";
        ui.label(RichText::new(scope).small().color(theme::muted(ui)));
        let other_accounts = crate::share::exec_run_as_supported();
        controls(ui, &mut dialog.draft, &provider.user_label, other_accounts);
        ui.separator();
        for note in programs_ui::notes(&dialog.draft, "Jedes Mitglied", provider, now) {
            ui.colored_label(theme::danger(ui), note);
        }
        let narrowing = "Laufende Befehle eines Mitglieds enden, wenn die Änderung seine \
                             Rechte einschränkt.";
        ui.label(narrowing);
        let problem = dialog.draft.problem(provider, true, now);
        if let Some(reason) = &problem {
            ui.label(RichText::new(reason).color(theme::muted(ui)));
        }
        let allows = dialog.draft.decisions.iter().any(|(capability, decision)| !capability.requires_context() && decision.is_on());
        let label = if allows {
            format!("Gewählte Befehls- und Bildschirmrechte für {count} Mitglieder erlauben")
        } else {
            format!("Befehle, Terminal und Bildschirmrechte für {count} Mitglieder entziehen")
        };
        ui.horizontal_wrapped(|ui| {
            let confirm = egui::Button::new(RichText::new(label).strong());
            if ui.add_enabled(problem.is_none(), confirm).clicked() {
                result = Some(planned(&dialog, now));
                done = true;
            }
            done |= ui.button("Abbrechen").clicked();
        });
    });
    if done || !visible {
        close(ctx);
    } else {
        open(ctx, dialog);
    }
    result
}

/// Sends the change of every member; one notice or error for all of them.
fn apply_all(app: &mut App, views: &[ExecDeviceView], planned: Vec<Planned>) {
    let (mut stored, mut failed) = (0usize, Vec::new());
    for (key, change, revision) in planned {
        let Some(view) = views.iter().find(|view| view.target_key == key) else {
            failed.push(format!("{key}: nicht mehr Mitglied"));
            continue;
        };
        let target = view.target.clone();
        match crate::daemon::mutate_access_grant(target, change, Some(revision)) {
            Ok((result, _)) if result.persisted && result.applied && result.error.is_none() => {
                stored += 1;
            }
            Ok((result, _)) => failed.push(format!(
                "{}: gespeichert={}, angewendet={}",
                view.device_name, result.persisted, result.applied
            )),
            Err(error) => failed.push(format!("{}: {error}", view.device_name)),
        }
    }
    if failed.is_empty() {
        let text = format!("Zugriff für {stored} Mitglieder gespeichert");
        app.notice = Some((text, std::time::Instant::now()));
    } else {
        app.error_msg = Some(format!(
            "Zugriff für {stored} Mitglieder gespeichert, nicht für: {}. Nächster Schritt: \
             „Zugriff für alle Mitglieder…“ erneut öffnen; der Dialog zeigt den aktuellen Stand.",
            failed.join("; ")
        ));
    }
    app.share_next_poll_at = std::time::Instant::now();
}
