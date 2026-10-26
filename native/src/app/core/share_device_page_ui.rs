//! Page „Geräte“: one line per device (direct contacts, and authorizations
//! of devices without a contact) and the detail page of one device with all
//! its settings: connection (automatisch verbinden/öffnen, Vertrauen
//! zurücksetzen), my exports for it (Rückfreigabe, Schreibrecht,
//! Autorisierung) and its access (Zugriff: commands, terminal, account, end;
//! „Ändern…“ opens the access dialog). „Gerät hinzufügen“ (PIN or
//! Direkt-Code) and „Entfernte Geräte“ are pages of their own.
use super::pages_ui::{self, SharePage, TAB_DEVICES, TAB_REQUESTS};
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    match pages_ui::current(ui, TAB_DEVICES) {
        SharePage::Contact(contact_id) => contact_page(app, ui, &contact_id),
        SharePage::Grant(device_id) => grant_page(app, ui, &device_id),
        SharePage::Device {
            device_id,
            public_key,
        } => pinned_page(app, ui, &device_id, &public_key),
        SharePage::AddDevice => {
            if pages_ui::back_row(ui, "Geraet hinzufuegen") {
                pages_ui::back(ui);
                return;
            }
            pages_ui::heading(ui, "PER PIN: AUFFINDBAR MACHEN ODER GERAET FINDEN");
            app.ui_share_discovery_direct(ui);
            ui.separator();
            pages_ui::heading(ui, "PER DIREKT-CODE DES ANDEREN GERAETS");
            app.ui_share_add_device(ui);
        }
        SharePage::RemovedDevices => {
            if pages_ui::back_row(ui, "Entfernte Geraete") {
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

/// „Schreibrecht · Zugriff: Befehle: erlaubt · …“ of an authorization.
fn rights_line(app: &App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) -> String {
    let mut parts = vec![if grant.write {
        "Schreibrecht".to_string()
    } else {
        "nur lesen".to_string()
    }];
    let key = crate::app::share_exec_ui::direct_target_key(&grant.device_id, &grant.fingerprint);
    if let Some(access) = crate::app::share_exec_ui::exec_summary(app, ui, &key) {
        parts.push(format!("Zugriff: {access}"));
    }
    parts.join(" · ")
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
            if ui.button("Anfragen oeffnen").clicked() {
                next = Some((TAB_REQUESTS, SharePage::Overview));
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Geraet hinzufuegen …").clicked() {
            next = Some((TAB_DEVICES, SharePage::AddDevice));
        }
        if removed > 0
            && ui
                .button(format!("Entfernte Geraete ({removed}) …"))
                .clicked()
        {
            next = Some((TAB_DEVICES, SharePage::RemovedDevices));
        }
    });
    pages_ui::heading(ui, "MEINE GERAETE");
    let profiles = &app.share_profiles;
    let unlinked: Vec<&crate::share::DirectGrant> = profiles
        .direct_grants
        .iter()
        .filter(|grant| !has_contact(profiles, grant))
        .collect();
    if profiles.direct_contacts.is_empty() && unlinked.is_empty() {
        ui.label(
            "Noch kein Geraet verbunden. „Geraet hinzufuegen“ koppelt per PIN oder Direkt-Code.",
        );
    }
    for contact in &profiles.direct_contacts {
        let rights = grant_of(profiles, contact).map(|grant| rights_line(app, ui, grant));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&contact.display_name).strong());
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    contact.status.label(),
                    contact.access_state.label()
                ))
                .small()
                .color(theme::muted(ui)),
            );
            if let Some(rights) = &rights {
                ui.label(RichText::new(rights).small());
            }
            if ui.button("Oeffnen").clicked() {
                open_target = Some(crate::share::PeerOpenTarget::Direct {
                    contact_id: contact.id.clone(),
                });
            }
            if ui.button("Details").clicked() {
                next = Some((TAB_DEVICES, SharePage::Contact(contact.id.clone())));
            }
        });
    }
    for grant in unlinked {
        let rights = rights_line(app, ui, grant);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&grant.device_name).strong());
            ui.label(
                RichText::new("nur Zugriff auf meine Freigaben")
                    .small()
                    .color(theme::muted(ui)),
            );
            ui.label(RichText::new(rights).small());
            if ui.button("Details").clicked() {
                next = Some((TAB_DEVICES, SharePage::Grant(grant.device_id.clone())));
            }
        });
    }
    // A room member has a device page even without a saved Direct contact.
    let mut listed = std::collections::BTreeSet::new();
    for room in &app.share_profiles.rooms {
        for member in &room.members {
            let identity = (member.device_id.clone(), member.public_key.clone());
            let direct = app.share_profiles.direct_grants.iter().any(|grant| {
                grant.device_id == member.device_id && grant.public_key == member.public_key
            }) || app.share_profiles.direct_contacts.iter().any(|contact| {
                contact.remote_device_id.as_deref() == Some(member.device_id.as_str())
                    && contact.remote_public_key.as_deref() == Some(member.public_key.as_str())
            });
            if direct || !listed.insert(identity.clone()) {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if member.device_name.is_empty() {
                        "Gerät ohne Namen"
                    } else {
                        &member.device_name
                    })
                    .strong(),
                );
                ui.label("Raummitglied");
                if ui.button("Geräteeinstellungen …").clicked() {
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
        if pages_ui::back_row(ui, "Geraet") {
            pages_ui::back(ui);
        }
        ui.label("Dieses Geraet ist nicht mehr gespeichert.");
        return;
    };
    if pages_ui::back_row(ui, &contact.display_name) {
        pages_ui::back(ui);
        return;
    }
    connection_section(app, ui, &contact);
    screen_section(app, ui, &contact);
    if let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key) {
        crate::app::share_rdp_ui::device(ui, &app.share_profiles, device, key);
        crate::app::share_apps_ui::device(ui, &app.share_profiles, device, key);
        crate::app::family_device_ui::device(ui, &app.share_profiles, device, key);
        crate::app::share_app_grant_ui::device(ui, device, key);
        crate::app::share_login_ui::device(ui, device, key);
        crate::app::share_tcp_ui::device(ui, &app.share_profiles, device, key);
        crate::app::share_routes_ui::device(ui, app.share_routes.as_ref(), device, key);
    }
    pages_ui::heading(ui, "MEINE FREIGABEN FUER DIESES GERAET");
    let mut share_back = contact.relation.share_back;
    if ui
        .checkbox(
            &mut share_back,
            "Auch meine Freigaben fuer dieses Geraet oeffnen",
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
    let grant = grant_of(&app.share_profiles, &contact).cloned();
    match grant {
        Some(grant) => grant_section(app, ui, &grant),
        None => {
            ui.label("Dieses Gerät hat noch keine Direktfreigabe für meine Dateien.");
            if let (Some(device), Some(key)) =
                (&contact.remote_device_id, &contact.remote_public_key)
            {
                crate::app::device_rights_ui::ui(app, ui, device, key);
            }
        }
    }
    pages_ui::heading(ui, "ENTFERNEN");
    if ui
        .button("Geraet entfernen")
        .on_hover_text(
            "Loescht Kontakt, Autorisierung und Anfragen dieses Geraets. Es kann sich erst nach einer erneuten bewussten Kopplung wieder verbinden.",
        )
        .clicked()
        && app.remove_direct_peer_completely(&contact.id)
    {
        pages_ui::back(ui);
    }
}

fn connection_section(app: &mut App, ui: &mut egui::Ui, contact: &crate::share::DirectContact) {
    pages_ui::heading(ui, "VERBINDUNG");
    ui.label(format!(
        "Status: {} · Zugriff: {}",
        contact.status.label(),
        contact.access_state.label()
    ));
    if let Some(error) = &contact.last_error {
        ui.colored_label(theme::danger(ui), error);
    }
    let mut form = crate::app::settings_form::Form::new(
        ui,
        "contact_form",
        &[
            "Fingerabdruck",
            "Automatisch verbinden",
            "Automatisch öffnen",
        ],
    );
    form.row(ui, "Fingerabdruck", |ui| {
        ui.horizontal_wrapped(|ui| {
            share_value_field(ui, &contact.expected_fingerprint);
            if ui.button("Kopieren").clicked() {
                ui.ctx().copy_text(contact.expected_fingerprint.clone());
            }
        });
    });
    let mut open = false;
    let mut request = false;
    let mut diagnose = false;
    ui.horizontal_wrapped(|ui| {
        open = ui.button("Oeffnen").clicked();
        request = ui
            .add_enabled(
                contact.access_state != crate::share::DirectAccessState::Accepted,
                egui::Button::new("Zugriff anfragen"),
            )
            .clicked();
        diagnose = ui.button("Diagnose").clicked();
    });
    let mut auto_connect = contact.auto_connect;
    let mut auto_open = contact.auto_open;
    let connect_changed = form.row_labelled(ui, "Automatisch verbinden", |ui, label| {
        ui.checkbox(&mut auto_connect, "Aktiv")
            .labelled_by(label)
            .changed()
    });
    let open_changed = form.row_labelled(ui, "Automatisch öffnen", |ui, label| {
        ui.checkbox(&mut auto_open, "Sobald erreichbar")
            .labelled_by(label)
            .changed()
    });
    if connect_changed || open_changed {
        persist(app, "Verbindungseinstellung", |profiles| {
            profiles.set_contact_options(
                &contact.id,
                connect_changed.then_some(auto_connect),
                open_changed.then_some(auto_open),
            )
        });
    }
    if ui
        .button("Vertrauen zuruecksetzen")
        .on_hover_text(
            "Vergisst die gepinnte Identitaet dieses Geraets und entzieht seine Autorisierung (Exec aus); der Kontakt bleibt und muss neu bestaetigt werden.",
        )
        .clicked()
    {
        let now = crate::share::core_now_secs();
        persist(app, "Vertrauen", |profiles| {
            profiles.reset_contact_trust(&contact.id, now)
        });
    }
    if diagnose {
        let presence = contact
            .presence
            .as_ref()
            .map(|p| {
                format!(
                    "node={}, relay={}, candidates={:?}, expires_at={}",
                    p.node_id, p.relay_url, p.candidates, p.expires_at
                )
            })
            .unwrap_or_else(|| "keine Presence".to_string());
        app.append_share_diag(format!(
            "Direct {}: lookup={}, fp={}, status={}, {}\n",
            contact.display_name,
            contact.lookup_id,
            contact.expected_fingerprint,
            contact.status.label(),
            presence
        ));
        app.share_tab = pages_ui::TAB_DIAGNOSTICS;
    }
    if request {
        let _ = super::lifecycle_ui::queue_contact(app, &contact.id);
    }
    if open {
        app.open_share_target(crate::share::PeerOpenTarget::Direct {
            contact_id: contact.id.clone(),
        });
    }
}

/// „Bildschirm öffnen“ and „Bildschirm steuern“ (SCR-M7): the viewer window.
fn screen_section(app: &mut App, ui: &mut egui::Ui, contact: &crate::share::DirectContact) {
    pages_ui::heading(ui, "BILDSCHIRM");
    let accepted = contact.access_state == crate::share::DirectAccessState::Accepted;
    let device_id = contact.remote_device_id.as_deref();
    let mut launch = None;
    ui.horizontal_wrapped(|ui| {
        for button in crate::app::screen_launch::device_buttons(device_id, accepted) {
            if crate::app::screen_launch::show_button(ui, &button) {
                launch = Some(button.control);
            }
        }
    });
    if let Some(control) = launch {
        let mut target = crate::app::screen_target::ScreenTarget::direct(&contact.id, device_id);
        target.control = control;
        if let Err(error) = crate::app::screen_launch::open_view(&target) {
            app.error_msg = Some(error);
        }
    }
}

fn grant_page(app: &mut App, ui: &mut egui::Ui, device_id: &str) {
    let Some(grant) = app
        .share_profiles
        .direct_grants
        .iter()
        .find(|grant| grant.device_id == device_id)
        .cloned()
    else {
        if pages_ui::back_row(ui, "Geraet") {
            pages_ui::back(ui);
        }
        ui.label("Fuer dieses Geraet gibt es keine Autorisierung mehr.");
        return;
    };
    if pages_ui::back_row(ui, &grant.device_name) {
        pages_ui::back(ui);
        return;
    }
    ui.label("Dieses Geraet hat Zugriff auf meine Freigaben, ist aber nicht in meiner Geraeteliste gespeichert.");
    grant_section(app, ui, &grant);
    crate::app::share_rdp_ui::device(ui, &app.share_profiles, &grant.device_id, &grant.public_key);
    crate::app::share_apps_ui::device(ui, &app.share_profiles, &grant.device_id, &grant.public_key);
    crate::app::family_device_ui::device(
        ui,
        &app.share_profiles,
        &grant.device_id,
        &grant.public_key,
    );
    crate::app::share_app_grant_ui::device(ui, &grant.device_id, &grant.public_key);
    crate::app::share_login_ui::device(ui, &grant.device_id, &grant.public_key);
    crate::app::share_tcp_ui::device(ui, &app.share_profiles, &grant.device_id, &grant.public_key);
    crate::app::share_routes_ui::device(
        ui,
        app.share_routes.as_ref(),
        &grant.device_id,
        &grant.public_key,
    );
}

/// Authorization, write right and commands of one authorized device.
fn grant_section(app: &mut App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) {
    pages_ui::heading(ui, "DIREKTFREIGABE FÜR DATEIEN");
    super::lifecycle_ui::ui_authorized_device(app, ui, &grant.device_id);
    let mut write = grant.write;
    let editable = grant.write || grant.state == crate::share::DirectGrantState::Accepted;
    if ui
        .add_enabled(editable, egui::Checkbox::new(&mut write, "Darf schreiben"))
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
        ui.label("Freigabe inaktiv; zuerst ausdruecklich wieder erlauben.");
    }
    pages_ui::heading(ui, "ALLE RECHTE DIESES GERÄTS");
    crate::app::device_rights_ui::ui(app, ui, &grant.device_id, &grant.public_key);
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
    crate::app::device_rights_ui::ui(app, ui, device_id, public_key);
    crate::app::share_rdp_ui::device(ui, &app.share_profiles, device_id, public_key);
    crate::app::share_apps_ui::device(ui, &app.share_profiles, device_id, public_key);
    crate::app::family_device_ui::device(ui, &app.share_profiles, device_id, public_key);
    crate::app::share_app_grant_ui::device(ui, device_id, public_key);
    crate::app::share_login_ui::device(ui, device_id, public_key);
    crate::app::share_tcp_ui::device(ui, &app.share_profiles, device_id, public_key);
    crate::app::share_routes_ui::device(ui, app.share_routes.as_ref(), device_id, public_key);
}
