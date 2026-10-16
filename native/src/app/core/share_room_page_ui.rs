//! Page „Räume“: one line per saved room and the detail page of one room
//! with all its settings (beitreten/verlassen, Name, Code, Freigaben, Rechte,
//! Mitglieder zulassen/sperren, Befehle eines Mitglieds). Creating, joining
//! and discovering rooms is a page of its own.
use super::pages_ui::{self, SharePage, TAB_EXEC, TAB_ROOMS};
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    match pages_ui::current(ui, TAB_ROOMS) {
        SharePage::Room(profile_id) => room_page(app, ui, &profile_id),
        SharePage::AddRoom => {
            if pages_ui::back_row(ui, "Raum erstellen oder beitreten") {
                pages_ui::back(ui);
                return;
            }
            app.ui_share_room_forms(ui);
        }
        _ => overview(app, ui),
    }
}

fn pending_members(room: &crate::share::RoomProfile) -> usize {
    room.members
        .iter()
        .filter(|member| member.relation.admission == crate::share::RoomMemberAdmission::Pending)
        .count()
}

fn overview(app: &mut App, ui: &mut egui::Ui) {
    let mut next = None;
    if ui.button("Raum erstellen oder beitreten …").clicked() {
        next = Some(SharePage::AddRoom);
    }
    pages_ui::heading(ui, "GESPEICHERTE RAEUME");
    if app.share_profiles.rooms.is_empty() {
        ui.label(
            "Noch keine Raeume. Ein Raum verbindet mehrere Geraete ueber einen gemeinsamen Code.",
        );
    }
    for room in &app.share_profiles.rooms {
        let pending = pending_members(room);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&room.name).strong());
            let state = if room.auto_join {
                room.status.label().to_string()
            } else {
                "verlassen".to_string()
            };
            ui.label(
                RichText::new(format!("{state} · Mitglieder: {}", room.members.len()))
                    .small()
                    .color(theme::muted(ui)),
            );
            if pending > 0 {
                ui.colored_label(
                    theme::warning(ui),
                    format!("{pending} warten auf Zulassung"),
                );
            }
            if ui.button("Details").clicked() {
                next = Some(SharePage::Room(room.id.clone()));
            }
        });
    }
    if let Some(page) = next {
        pages_ui::open(app, ui, TAB_ROOMS, page);
    }
}

fn room_page(app: &mut App, ui: &mut egui::Ui, profile_id: &str) {
    let Some(room) = app
        .share_profiles
        .rooms
        .iter()
        .find(|room| room.id == profile_id)
        .cloned()
    else {
        if pages_ui::back_row(ui, "Raum") {
            pages_ui::back(ui);
        }
        ui.label("Dieser Raum ist nicht mehr gespeichert.");
        return;
    };
    if pages_ui::back_row(ui, &room.name) {
        pages_ui::back(ui);
        return;
    }
    pages_ui::heading(ui, "RAUM");
    ui.label(format!(
        "Status: {}",
        if room.auto_join {
            room.status.label().to_string()
        } else {
            "verlassen (tritt nicht automatisch bei)".to_string()
        }
    ));
    let mut remove = false;
    ui.horizontal_wrapped(|ui| {
        if room.auto_join {
            if ui.button("Verlassen").clicked() {
                persist(app, "Raum", |profiles| {
                    profiles.set_room_auto_join(&room.id, false)
                });
                let _ = app.share_cmd(crate::share::ShareCmd::LeaveRoom {
                    room_id: room.room_id.clone(),
                });
            }
        } else if ui.button("Wieder beitreten").clicked() {
            persist(app, "Raum", |profiles| {
                profiles.set_room_auto_join(&room.id, true)
            });
        }
        if ui.button("Code kopieren").clicked() {
            match crate::share::ShareProfiles::room_code_checked(&room) {
                Ok(Some(code)) => ui.ctx().copy_text(code),
                Ok(None) => app.error_msg = Some("Raum-Secret fehlt".into()),
                Err(error) => app.error_msg = Some(format!("Raum-Code lesen: {error}")),
            }
        }
        if ui.button("Freigaben dieses Raums").clicked() {
            app.share_export_scope = 2;
            app.share_export_target_id = room.id.clone();
            app.share_tab = pages_ui::TAB_EXPORTS;
        }
        remove = ui.button("Raum entfernen").clicked();
    });
    rename_row(app, ui, &room);
    pages_ui::heading(ui, "RECHTE");
    let mut may_write = room.policy.members_may_write;
    let mut confirm = room.policy.confirm_new_members;
    let write_changed = ui
        .checkbox(&mut may_write, "Mitglieder duerfen schreiben")
        .changed();
    let confirm_changed = ui
        .checkbox(&mut confirm, "Neue Mitglieder bestaetigen")
        .changed();
    ui.label("Ein Raum-Code gibt allein keine Ordner frei.");
    if write_changed || confirm_changed {
        persist(app, "Raumrechte", |profiles| {
            profiles.set_room_policy(
                &room.id,
                write_changed.then_some(may_write),
                confirm_changed.then_some(confirm),
            )
        });
    }
    pages_ui::heading(ui, "MITGLIEDER");
    if room.members.is_empty() {
        ui.label("Noch keine Mitglieder gesehen.");
    }
    for member in &room.members {
        member_row(app, ui, &room, member);
    }
    if remove && app.remove_room_completely(&room.id) {
        pages_ui::back(ui);
    }
}

fn rename_row(app: &mut App, ui: &mut egui::Ui, room: &crate::share::RoomProfile) {
    let id = egui::Id::new(("share_room_rename", &room.id));
    let mut name = ui.data_mut(|data| {
        data.get_temp::<String>(id)
            .unwrap_or_else(|| room.name.clone())
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Name:");
        if ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(share_input_width(ui, 220.0)))
            .changed()
        {
            ui.data_mut(|data| data.insert_temp(id, name.clone()));
        }
        if ui
            .add_enabled(name.trim() != room.name, egui::Button::new("Umbenennen"))
            .clicked()
        {
            persist(app, "Raumname", |profiles| {
                profiles.rename_room(&room.id, &name)
            });
            ui.data_mut(|data| data.remove::<String>(id));
        }
    });
}

#[derive(Clone, Copy)]
enum MemberAction {
    Admit,
    Block,
    Allow,
}

fn member_row(
    app: &mut App,
    ui: &mut egui::Ui,
    room: &crate::share::RoomProfile,
    member: &crate::share::RoomMember,
) {
    let pending = member.relation.admission == crate::share::RoomMemberAdmission::Pending;
    let state = if pending {
        "wartet auf Zulassung".to_string()
    } else if member.blocked {
        "gesperrt".to_string()
    } else {
        member.status.label().to_string()
    };
    let mut action = None;
    let mut open = false;
    let mut diagnose = false;
    let mut reset = false;
    let mut exec = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&member.device_name).strong());
            ui.label(RichText::new(&state).small().color(theme::muted(ui)));
        });
        ui.horizontal_wrapped(|ui| {
            if !member.blocked && !pending {
                open = ui.button("Oeffnen").clicked();
            }
            if pending {
                if ui.button("Zulassen").clicked() {
                    action = Some(MemberAction::Admit);
                }
            } else if member.blocked {
                if ui.button("Wieder zulassen").clicked() {
                    action = Some(MemberAction::Allow);
                }
            } else if ui.button("Sperren").clicked() {
                action = Some(MemberAction::Block);
            }
            if ui.button("Fingerprint kopieren").clicked() {
                ui.ctx().copy_text(member.fingerprint.clone());
            }
            diagnose = ui.button("Diagnose").clicked();
            reset = ui.button("Vertrauen zuruecksetzen").clicked();
            exec = ui.button("Befehle …").clicked();
        });
    });
    if let Some(action) = action {
        set_member(app, room, member, action);
    }
    if reset {
        persist(app, "Mitglied", |profiles| {
            profiles.reset_member_trust(&room.id, &member.device_id)
        });
    }
    if diagnose {
        let presence = member
            .presence
            .as_ref()
            .map(|p| format!("candidates={:?}, expires_at={}", p.candidates, p.expires_at))
            .unwrap_or_else(|| "keine Presence".to_string());
        app.append_share_diag(format!(
            "Raum {} / {}: fp={}, status={}, {}\n",
            room.name,
            member.device_name,
            member.fingerprint,
            member.status.label(),
            presence
        ));
        app.share_tab = pages_ui::TAB_DIAGNOSTICS;
    }
    if exec {
        let key = crate::app::share_exec_ui::room_target_key(
            &room.room_id,
            &member.device_id,
            &member.fingerprint,
        );
        pages_ui::open(app, ui, TAB_EXEC, SharePage::ExecTarget(key));
    }
    if open {
        app.open_share_target(crate::share::PeerOpenTarget::RoomDevice {
            room_id: room.id.clone(),
            device_id: member.device_id.clone(),
        });
    }
}

/// Admit, block or allow one member, only while its identity is the one
/// shown (as `share.setRoomMember` on Android).
fn set_member(
    app: &mut App,
    room: &crate::share::RoomProfile,
    member: &crate::share::RoomMember,
    action: MemberAction,
) {
    let now = crate::share::core_now_secs();
    persist(app, "Mitglied", |profiles| {
        let latest = profiles
            .rooms
            .iter_mut()
            .find(|candidate| candidate.id == room.id && candidate.room_id == room.room_id)
            .ok_or_else(|| "Raumidentitaet wurde geaendert; bitte neu laden".to_string())?;
        let mut matches = latest
            .members
            .iter()
            .filter(|candidate| candidate.device_id == member.device_id);
        let pinned = matches.next().is_some_and(|candidate| {
            candidate.public_key == member.public_key
                && candidate.node_id == member.node_id
                && candidate.fingerprint == member.fingerprint
        });
        if !pinned || matches.next().is_some() {
            return Err("Mitgliedsidentitaet wurde geaendert; bitte neu laden".into());
        }
        Ok(match action {
            MemberAction::Admit => latest.admit_member(&member.device_id),
            MemberAction::Block => latest.set_member_blocked(&member.device_id, true, now),
            MemberAction::Allow => latest.set_member_blocked(&member.device_id, false, now),
        })
    });
}

/// One profile transaction; the committed profiles replace the window's copy.
fn persist(
    app: &mut App,
    what: &str,
    mut edit: impl FnMut(&mut crate::share::ShareProfiles) -> Result<bool, String>,
) {
    let mut changed = false;
    match crate::share::ShareProfiles::mutate_persisted(home(), |profiles| {
        changed = edit(profiles)?;
        Ok(())
    }) {
        Ok(committed) => {
            app.share_profiles = committed;
            if changed {
                let _ = app.configure_share_service();
            }
        }
        Err(error) => app.error_msg = Some(format!("{what} nicht gespeichert: {error}")),
    }
}

fn home() -> Option<String> {
    Some(dirs_home().to_string_lossy().replace('\\', "/"))
}
