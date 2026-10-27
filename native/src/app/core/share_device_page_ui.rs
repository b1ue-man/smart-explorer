//! Page „Geräte“: one row per device (direct contacts, authorizations of
//! devices without a contact, room members) and the page of one device. The
//! device page has one card per area: what the device may do here (files,
//! screen, commands, own programs), what I use on it (its programs, windows,
//! Remote Desktop, TCP forwarding, machine login), the connection and
//! removal. „Gerät hinzufügen“ (PIN or Direkt-Code) and „Entfernte Geräte“
//! are pages of their own.
use super::pages_ui::{self, SharePage, TAB_DEVICES, TAB_REQUESTS};
use super::*;
use crate::app::device_page_layout as layout;

#[path = "share_device_connection_ui.rs"]
mod connection;
use connection::{connection_card, header};

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    match pages_ui::current(ui, TAB_DEVICES) {
        SharePage::Contact(contact_id) => contact_page(app, ui, &contact_id),
        SharePage::Grant(device_id) => grant_page(app, ui, &device_id),
        SharePage::Device {
            device_id,
            public_key,
        } => pinned_page(app, ui, &device_id, &public_key),
        SharePage::AddDevice => {
            if pages_ui::back_row(ui, "Gerät hinzufügen") {
                pages_ui::back(ui);
                return;
            }
            layout::card(
                ui,
                "Per PIN",
                Some("Dieses Gerät auffindbar machen oder ein Gerät in der Nähe finden"),
                |ui| {
                    app.ui_share_discovery_direct(ui);
                },
            );
            layout::card(
                ui,
                "Per Direkt-Code",
                Some("Den Direkt-Code des anderen Geräts eintragen"),
                |ui| {
                    app.ui_share_add_device(ui);
                },
            );
        }
        SharePage::RemovedDevices => {
            if pages_ui::back_row(ui, "Entfernte Geräte") {
                pages_ui::back(ui);
                return;
            }
            super::removed_devices_ui::ui(app, ui);
        }
        _ => overview(app, ui),
    }
}

/// The authorization of a contact: the grant of its pinned remote device.
fn grant_of<'a>(
    profiles: &'a crate::share::ShareProfiles,
    contact: &crate::share::DirectContact,
) -> Option<&'a crate::share::DirectGrant> {
    let device_id = contact.remote_device_id.as_deref()?;
    profiles.direct_grants.iter().find(|grant| {
        grant.device_id == device_id
            && contact
                .remote_public_key
                .as_deref()
                .is_none_or(|key| key == grant.public_key)
    })
}

fn has_contact(profiles: &crate::share::ShareProfiles, grant: &crate::share::DirectGrant) -> bool {
    profiles.direct_contacts.iter().any(|contact| {
        grant_of(profiles, contact).is_some_and(|own| own.device_id == grant.device_id)
    })
}

/// „Darf schreiben · Befehle: erlaubt · …“ of an authorization.
fn rights_line(app: &App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) -> String {
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

/// One device row: name, status, a muted second line, buttons on the right.
fn device_row(
    ui: &mut egui::Ui,
    name: &str,
    status: Option<(String, bool)>,
    detail: &str,
    add_buttons: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).strong());
        if let Some((text, reachable)) = status {
            layout::status_chip(ui, &text, reachable);
        }
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            add_buttons,
        );
    });
    if !detail.is_empty() {
        ui.label(RichText::new(detail).small().color(theme::muted(ui)));
    }
    ui.separator();
}

fn overview(app: &mut App, ui: &mut egui::Ui) {
    let requests = pages_ui::open_request_count(&app.share_profiles);
    let removed = app.share_profiles.removed_direct_peers.len();
    let mut next = None;
    let mut open_target = None;
    if requests > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                theme::warning(ui),
                format!("{requests} offene Anfrage(n) warten auf deine Entscheidung."),
            );
            if ui.button("Anfragen öffnen").clicked() {
                next = Some((TAB_REQUESTS, SharePage::Overview));
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Gerät hinzufügen …").clicked() {
            next = Some((TAB_DEVICES, SharePage::AddDevice));
        }
        if removed > 0
            && ui
                .button(format!("Entfernte Geräte ({removed}) …"))
                .clicked()
        {
            next = Some((TAB_DEVICES, SharePage::RemovedDevices));
        }
    });
    let now = crate::share::core_now_secs();
    let profiles = &app.share_profiles;
    let unlinked: Vec<&crate::share::DirectGrant> = profiles
        .direct_grants
        .iter()
        .filter(|grant| !has_contact(profiles, grant))
        .collect();
    let contact_rights: Vec<Option<String>> = profiles
        .direct_contacts
        .iter()
        .map(|contact| grant_of(profiles, contact).map(|grant| rights_line(app, ui, grant)))
        .collect();
    let grant_rights: Vec<String> = unlinked
        .iter()
        .map(|grant| rights_line(app, ui, grant))
        .collect();
    layout::card(ui, "Meine Geräte", None, |ui| {
        if profiles.direct_contacts.is_empty() && unlinked.is_empty() {
            ui.label(
                "Noch kein Gerät verbunden. „Gerät hinzufügen“ koppelt per PIN oder Direkt-Code.",
            );
        }
        for (contact, rights) in profiles.direct_contacts.iter().zip(&contact_rights) {
            let status = (
                contact.status.label(),
                crate::share::device_view::online(contact, now),
            );
            let detail = rights
                .clone()
                .unwrap_or_else(|| contact.access_state.label().to_string());
            device_row(ui, &contact.display_name, Some(status), &detail, |ui| {
                if ui.button("Einstellungen").clicked() {
                    next = Some((TAB_DEVICES, SharePage::Contact(contact.id.clone())));
                }
                if ui.button("Öffnen").clicked() {
                    open_target = Some(crate::share::PeerOpenTarget::Direct {
                        contact_id: contact.id.clone(),
                    });
                }
            });
        }
        for (grant, rights) in unlinked.iter().zip(&grant_rights) {
            let detail = format!("Nur Zugriff auf meine Freigaben · {rights}");
            device_row(ui, &grant.device_name, None, &detail, |ui| {
                if ui.button("Einstellungen").clicked() {
                    next = Some((TAB_DEVICES, SharePage::Grant(grant.device_id.clone())));
                }
            });
        }
        // A room member has a device page even without a saved Direct contact.
        let mut listed = std::collections::BTreeSet::new();
        for room in &profiles.rooms {
            for member in &room.members {
                let identity = (member.device_id.clone(), member.public_key.clone());
                let direct = profiles.direct_grants.iter().any(|grant| {
                    grant.device_id == member.device_id && grant.public_key == member.public_key
                }) || profiles.direct_contacts.iter().any(|contact| {
                    contact.remote_device_id.as_deref() == Some(member.device_id.as_str())
                        && contact.remote_public_key.as_deref() == Some(member.public_key.as_str())
                });
                if direct || !listed.insert(identity.clone()) {
                    continue;
                }
                let name = if member.device_name.is_empty() {
                    "Gerät ohne Namen"
                } else {
                    &member.device_name
                };
                let status = (
                    member.status.label(),
                    crate::share::device_view::reachable(&member.status),
                );
                let detail = format!("Mitglied im Raum „{}“", room.name);
                device_row(ui, name, Some(status), &detail, |ui| {
                    if ui.button("Einstellungen").clicked() {
                        next = Some((
                            TAB_DEVICES,
                            SharePage::Device {
                                device_id: identity.0,
                                public_key: identity.1,
                            },
                        ));
                    }
                });
            }
        }
    });
    if let Some((tab, page)) = next {
        pages_ui::open(app, ui, tab, page);
    }
    if let Some(target) = open_target {
        app.open_share_target(target);
    }
}

fn contact_page(app: &mut App, ui: &mut egui::Ui, contact_id: &str) {
    let Some(contact) = app
        .share_profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .cloned()
    else {
        if pages_ui::back_row(ui, "Gerät") {
            pages_ui::back(ui);
        }
        ui.label("Dieses Gerät ist nicht mehr gespeichert.");
        return;
    };
    if pages_ui::back_row(ui, &contact.display_name) {
        pages_ui::back(ui);
        return;
    }
    header(app, ui, &contact);
    let identity = contact
        .remote_device_id
        .clone()
        .zip(contact.remote_public_key.clone());
    let grant = grant_of(&app.share_profiles, &contact).cloned();
    layout::card(ui, "Was dieses Gerät bei mir darf", None, |ui| {
        let mut share_back = contact.relation.share_back;
        if ui
            .checkbox(
                &mut share_back,
                "Beim Verbinden auch meine Freigaben für dieses Gerät öffnen",
            )
            .changed()
        {
            match crate::share::set_direct_share_back(home(), &contact.id, share_back) {
                Ok(change) => {
                    app.share_profiles = change.profiles;
                    let _ = app.configure_share_service();
                }
                Err(error) => app.error_msg = Some(error),
            }
        }
        match &grant {
            Some(grant) => grant_rights(app, ui, grant),
            None => {
                ui.label("Dieses Gerät hat noch keine Direktfreigabe für meine Dateien.");
                if let Some((device, key)) = &identity {
                    crate::app::device_rights_ui::ui(app, ui, device, key);
                }
            }
        }
        if let Some((device, key)) = &identity {
            own_programs(ui, device, key);
        }
    });
    if let Some((device, key)) = &identity {
        uses_card(app, ui, device, key);
    }
    connection_card(app, ui, &contact);
    layout::card(ui, "Gerät entfernen", None, |ui| {
        if ui
            .button(RichText::new("Gerät entfernen").color(theme::danger(ui)))
            .on_hover_text(
                "Löscht Kontakt, Autorisierung und Anfragen dieses Geräts. Es kann sich erst nach einer erneuten bewussten Kopplung wieder verbinden.",
            )
            .clicked()
            && app.remove_direct_peer_completely(&contact.id)
        {
            pages_ui::back(ui);
        }
    });
}

fn grant_page(app: &mut App, ui: &mut egui::Ui, device_id: &str) {
    let Some(grant) = app
        .share_profiles
        .direct_grants
        .iter()
        .find(|grant| grant.device_id == device_id)
        .cloned()
    else {
        if pages_ui::back_row(ui, "Gerät") {
            pages_ui::back(ui);
        }
        ui.label("Für dieses Gerät gibt es keine Autorisierung mehr.");
        return;
    };
    if pages_ui::back_row(ui, &grant.device_name) {
        pages_ui::back(ui);
        return;
    }
    ui.label(
        RichText::new("Dieses Gerät hat Zugriff auf meine Freigaben, ist aber nicht in meiner Geräteliste gespeichert.")
            .color(theme::muted(ui)),
    );
    layout::card(ui, "Was dieses Gerät bei mir darf", None, |ui| {
        grant_rights(app, ui, &grant);
        own_programs(ui, &grant.device_id, &grant.public_key);
    });
    uses_card(app, ui, &grant.device_id, &grant.public_key);
    layout::card(ui, "Verbindung", None, |ui| {
        layout::tool(
            ui,
            ("route", &grant.device_id),
            "Verbindungsweg (direkt oder Relay)",
            |ui| {
                crate::app::share_routes_ui::device(
                    ui,
                    app.share_routes.as_ref(),
                    &grant.device_id,
                    &grant.public_key,
                );
            },
        );
    });
}

/// Files (authorization, write right), then screen and commands.
fn grant_rights(app: &mut App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) {
    ui.label(RichText::new("Dateien").strong());
    super::lifecycle_ui::ui_authorized_device(app, ui, &grant.device_id);
    let mut write = grant.write;
    let editable = grant.write || grant.state == crate::share::DirectGrantState::Accepted;
    if ui
        .add_enabled(
            editable,
            egui::Checkbox::new(&mut write, "Darf in meine Freigaben schreiben"),
        )
        .changed()
    {
        let peer = crate::share::DirectPeerIdentity {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            public_key: grant.public_key.clone(),
            node_id: grant.node_id.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        match crate::share::set_direct_peer_write(home(), &peer, write) {
            Ok(change) => {
                app.share_profiles = change.profiles;
                let _ = app.configure_share_service();
            }
            Err(error) => app.error_msg = Some(format!("Schreibrecht: {error}")),
        }
    }
    if !editable {
        ui.label("Freigabe inaktiv; zuerst ausdrücklich wieder erlauben.");
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Bildschirm und Befehle").strong());
    crate::app::device_rights_ui::ui(app, ui, &grant.device_id, &grant.public_key);
}

/// My own programs this device may start here.
fn own_programs(ui: &mut egui::Ui, device: &str, key: &str) {
    ui.add_space(6.0);
    layout::tool(
        ui,
        ("own-apps", device),
        "Eigene Programme für dieses Gerät",
        |ui| {
            crate::app::share_app_grant_ui::device(ui, device, key);
        },
    );
}

/// What I use on the other device.
fn uses_card(app: &App, ui: &mut egui::Ui, device: &str, key: &str) {
    let profiles = &app.share_profiles;
    layout::card(ui, "Was ich auf diesem Gerät nutze", None, |ui| {
        layout::tool(ui, ("apps", device), "Programme", |ui| {
            ui.small("Startet Programme des anderen Geräts. Dafür muss es dir dort unter „Eigene Programme für dieses Gerät“ Programme freigeben.");
            crate::app::share_apps_ui::device(ui, profiles, device, key);
        });
        layout::tool(ui, ("windows", device), "Fenster", |ui| {
            ui.small("Zeigt einzelne Fenster des anderen Geräts. Dafür muss es dir „Bildschirm ansehen“ erlauben.");
            crate::app::family_device_ui::device(ui, profiles, device, key);
        });
        layout::tool(ui, ("rdp", device), "Remote Desktop / RemoteApp", |ui| {
            crate::app::share_rdp_ui::device(ui, profiles, device, key);
        });
        layout::tool(ui, ("tcp", device), "TCP-Weiterleitungen", |ui| {
            crate::app::share_tcp_ui::device(ui, profiles, device, key);
        });
        layout::tool(ui, ("login", device), "Maschinenanmeldung", |ui| {
            crate::app::share_login_ui::device(ui, device, key);
        });
    });
}

/// One profile transaction with an edit of `crate::share::ShareProfiles`;
/// the committed profiles replace the window's copy.
fn persist(
    app: &mut App,
    what: &str,
    edit: impl Fn(&mut crate::share::ShareProfiles) -> Result<bool, String>,
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

fn pinned_page(app: &mut App, ui: &mut egui::Ui, device_id: &str, public_key: &str) {
    let contact = app
        .share_profiles
        .direct_contacts
        .iter()
        .find(|contact| {
            contact.remote_device_id.as_deref() == Some(device_id)
                && contact.remote_public_key.as_deref() == Some(public_key)
        })
        .map(|contact| contact.id.clone());
    if let Some(contact) = contact {
        contact_page(app, ui, &contact);
        return;
    }
    if app
        .share_profiles
        .direct_grants
        .iter()
        .any(|grant| grant.device_id == device_id && grant.public_key == public_key)
    {
        grant_page(app, ui, device_id);
        return;
    }
    let name = app
        .share_profiles
        .rooms
        .iter()
        .flat_map(|room| &room.members)
        .find(|member| member.device_id == device_id && member.public_key == public_key)
        .map(|member| member.device_name.clone())
        .unwrap_or_else(|| "Gerät ohne Namen".into());
    if pages_ui::back_row(ui, &name) {
        pages_ui::back(ui);
        return;
    }
    layout::card(ui, "Was dieses Gerät bei mir darf", None, |ui| {
        crate::app::device_rights_ui::ui(app, ui, device_id, public_key);
        own_programs(ui, device_id, public_key);
    });
    uses_card(app, ui, device_id, public_key);
    layout::card(ui, "Verbindung", None, |ui| {
        layout::tool(
            ui,
            ("route", device_id),
            "Verbindungsweg (direkt oder Relay)",
            |ui| {
                crate::app::share_routes_ui::device(
                    ui,
                    app.share_routes.as_ref(),
                    device_id,
                    public_key,
                );
            },
        );
    });
}
