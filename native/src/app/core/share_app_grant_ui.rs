//! First AppLaunch grant from this host's local, daemon-token protected catalog.
use crate::{
    apps::{AppId, LocalAppGrantChange, LocalAppGrantSnapshot},
    share::{
        access_grant::{Capability, CapabilityDecision},
        access_relation_view::AccessRelationView,
        ExecGrantTarget,
    },
};
use eframe::egui;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Default)]
struct State {
    snapshot: Option<LocalAppGrantSnapshot>,
    source: Option<ExecGrantTarget>,
    selected: BTreeSet<AppId>,
    decision: CapabilityDecision,
    expiry: usize,
    old_end: Option<i64>,
    search: String,
    page: usize,
    rebind: bool,
    ack: bool,
    busy: bool,
    fresh: bool,
    error: Option<String>,
    info: Option<String>,
}
type Shared = Arc<Mutex<State>>;
fn select(state: &mut State, source: &AccessRelationView) {
    state.source = Some(source.target.clone());
    state.selected.clear();
    state.ack = false;
    state.expiry = 1;
    state.old_end = None;
    state.decision = source
        .grant
        .capabilities
        .get(&Capability::AppLaunch)
        .copied()
        .unwrap_or_default();
    if let Some(bounds) = &source.grant.constraints.app_launch {
        state.old_end = bounds.valid_until;
        state.expiry = 0;
        if let Some(snapshot) = &state.snapshot {
            state.selected.extend(
                snapshot
                    .catalog
                    .apps
                    .iter()
                    .filter(|app| {
                        bounds
                            .app_ids
                            .iter()
                            .any(|id| id.as_str() == app.app_id.as_str())
                    })
                    .map(|app| app.app_id.clone()),
            );
            state.rebind = bounds.sessions.len() != 1
                || bounds.sessions[0].as_str() != snapshot.catalog.authorization_scope.as_str()
                || bounds.users.len() != 1
                || bounds.users[0].as_str() != snapshot.catalog.session.user_id.as_str()
                || state.selected.len() != bounds.app_ids.len();
        }
    } else {
        state.rebind = false;
    }
}
fn install(state: &mut State, snapshot: LocalAppGrantSnapshot) {
    let source = state
        .source
        .as_ref()
        .and_then(|target| snapshot.relations.iter().find(|r| r.target == *target))
        .cloned();
    state.snapshot = Some(snapshot);
    state.fresh = true;
    if let Some(source) = source {
        select(state, &source);
    } else {
        state.source = None;
        state.selected.clear();
        state.ack = false;
    }
}
pub(crate) fn device(ui: &mut egui::Ui, device: &str, key: &str) {
    ui.separator();
    ui.heading("Eigene Programme für dieses Gerät freigeben");
    ui.label("Wähle eine einzelne Direkt- oder Raumfreigabe. Die Appauswahl stammt aus der aktuellen Anmeldung auf diesem Host.");
    let id = egui::Id::new(("local-app-grants", device, key));
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    let refresh;
    let mut change = None;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        refresh = ui
            .add_enabled(
                !state.busy,
                egui::Button::new("Lokalen Appkatalog und Rechtequellen laden / aktualisieren"),
            )
            .clicked();
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Lokale Appfreigabe wird verarbeitet …");
            });
        }
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(info) = &state.info {
            ui.label(info);
        }
        if let Some(snapshot) = state.snapshot.clone() {
            if !state.fresh {
                ui.label("Katalog oder Rechtequelle sind nicht mehr aktuell. Vor dem Erlauben oder Fragen erneut laden; Ausschalten bleibt möglich.");
            }
            if snapshot.peer_device_id != device || snapshot.peer_public_key != key {
                ui.label("Geräteidentität geändert; Appfreigabe neu öffnen.");
                return;
            }
            ui.label(format!(
                "Lokaler Appbenutzer: {}",
                snapshot.catalog.session.user_id
            ));
            if snapshot.relations.is_empty() {
                ui.label("Keine eingehende Freigabequelle für diese Geräteidentität. Zuerst die Direktbeziehung oder das Raummitglied zulassen.");
            }
            for source in &snapshot.relations {
                let label = if source.relation_kind == "direct" {
                    format!("Direktfreigabe ({})", source.relation_id)
                } else {
                    format!("Raum „{}“ ({})", source.relation_name, source.relation_id)
                };
                if ui
                    .add_enabled(
                        !state.busy,
                        egui::SelectableLabel::new(
                            state.source.as_ref() == Some(&source.target),
                            label,
                        ),
                    )
                    .clicked()
                {
                    select(&mut state, source);
                    state.error = None;
                    state.info = None;
                }
            }
            if let Some(source) = state
                .source
                .as_ref()
                .and_then(|target| snapshot.relations.iter().find(|r| r.target == *target))
                .cloned()
            {
                ui.label(format!("Policyrevision: {}", source.policy_revision));
                if !source.active {
                    ui.label("Diese Beziehung ist inaktiv. Appstarts können ausschließlich ausgeschaltet werden.");
                }
                if source.grant.expired {
                    ui.label("Die übergeordnete Freigabe ist abgelaufen. Ihre gemeinsame Frist bleibt erhalten und begrenzt auch Appstarts.");
                }
                let source_changed = state.source.clone();
                egui::ComboBox::from_id_salt(id.with("decision"))
                    .selected_text(decision_name(state.decision))
                    .show_ui(ui, |ui| {
                        for decision in [
                            CapabilityDecision::Off,
                            CapabilityDecision::Ask,
                            CapabilityDecision::Allow,
                        ] {
                            ui.selectable_value(
                                &mut state.decision,
                                decision,
                                decision_name(decision),
                            );
                        }
                    });
                if state.decision == CapabilityDecision::Off {
                    ui.small("Aus entzieht ausschließlich Appstarts. Bisherige Appgrenzen bleiben gespeichert; die neue Auswahl gilt erst beim ausdrücklichen Aktivieren.");
                }
                if state.rebind {
                    ui.colored_label(egui::Color32::YELLOW,"Anmeldung, Katalog oder bisherige Appmenge hat sich geändert. Die bisherigen Rechte gelten nicht für den neuen Katalog.");
                    ui.checkbox(&mut state.ack,"Die unten ausgewählten App-IDs ausdrücklich an diese aktuelle Anmeldung und diesen Katalog binden");
                }
                let expiry_labels = [
                    "Bisherige Appgültigkeit beibehalten",
                    "15 Minuten ab Speichern",
                    "1 Stunde ab Speichern",
                    "8 Stunden ab Speichern",
                    "1 Tag ab Speichern",
                    "Unbegrenzt für diese Anmeldung und diesen Katalog",
                ];
                egui::ComboBox::from_id_salt(id.with("expiry"))
                    .selected_text(expiry_labels[state.expiry])
                    .show_ui(ui, |ui| {
                        for (index, label) in expiry_labels.iter().enumerate() {
                            ui.selectable_value(&mut state.expiry, index, *label);
                        }
                    });
                ui.small(format!(
                    "Bisheriges eigenes Ende: {}",
                    super::device_rights_model::expiry(state.old_end)
                ));
                if ui.text_edit_singleline(&mut state.search).changed() {
                    state.page = 0;
                }
                let needle = state.search.to_lowercase();
                let visible: Vec<_> = snapshot
                    .catalog
                    .apps
                    .iter()
                    .filter(|app| {
                        app.name.to_lowercase().contains(&needle)
                            || app.app_id.as_str().to_lowercase().contains(&needle)
                    })
                    .collect();
                let pages = visible.len().div_ceil(12).max(1);
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
                        visible.len()
                    ));
                    if ui
                        .add_enabled(state.page + 1 < pages, egui::Button::new("Weiter"))
                        .clicked()
                    {
                        state.page += 1;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt(id.with("apps"))
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for app in visible.iter().skip(state.page * 12).take(12) {
                            let mut checked = state.selected.contains(&app.app_id);
                            if ui
                                .add_enabled(
                                    !state.busy && (app.can_launch || checked),
                                    egui::Checkbox::new(&mut checked, &app.name),
                                )
                                .changed()
                            {
                                if checked {
                                    state.selected.insert(app.app_id.clone());
                                } else {
                                    state.selected.remove(&app.app_id);
                                }
                            }
                            if !app.can_launch {
                                ui.small(format!(
                                    "Start nicht verfügbar: {}",
                                    app.unavailable_reason
                                        .as_deref()
                                        .unwrap_or("Keine reale Startfähigkeit bestätigt")
                                ));
                            }
                            ui.small(app.app_id.as_str());
                        }
                    });
                ui.label(format!("{} konkrete App-IDs ausgewählt. Neu installierte Programme werden nicht automatisch aufgenommen.",state.selected.len()));
                let end = match state.expiry {
                    0 => state.old_end,
                    1 => Some(crate::share::core_now_secs() + 900),
                    2 => Some(crate::share::core_now_secs() + 3600),
                    3 => Some(crate::share::core_now_secs() + 28800),
                    4 => Some(crate::share::core_now_secs() + 86400),
                    _ => None,
                };
                let on = state.decision != CapabilityDecision::Off;
                let selected_available = state.selected.iter().all(|id| {
                    snapshot
                        .catalog
                        .apps
                        .iter()
                        .any(|app| app.app_id == *id && app.can_launch)
                });
                let available = !state.busy
                    && (!on
                        || (state.fresh
                            && state.error.is_none()
                            && source.active
                            && !state.selected.is_empty()
                            && selected_available
                            && (!state.rebind || state.ack)
                            && end.is_none_or(|end| end > crate::share::core_now_secs())));
                if ui
                    .add_enabled(
                        available,
                        egui::Button::new(
                            "Appstartfreigabe ausschließlich für diese Quelle speichern",
                        ),
                    )
                    .clicked()
                {
                    change = source_changed.map(|target| LocalAppGrantChange {
                        target,
                        expected_policy_revision: source.policy_revision,
                        host_device_id: snapshot.catalog.device_id.clone(),
                        catalog_revision: snapshot.catalog.revision.clone(),
                        user_id: snapshot.catalog.session.user_id.clone(),
                        authorization_scope: snapshot.catalog.authorization_scope.clone(),
                        app_ids: state.selected.iter().cloned().collect(),
                        decision: state.decision,
                        valid_until: end,
                    });
                }
            }
        }
    }
    if refresh {
        load(ui.ctx(), &shared, device.into(), key.into());
    }
    if let Some(change) = change {
        save(ui.ctx(), &shared, device.into(), key.into(), change);
    }
}
fn decision_name(decision: CapabilityDecision) -> &'static str {
    match decision {
        CapabilityDecision::Off => "Aus",
        CapabilityDecision::Ask => "Vor jedem Start fragen",
        CapabilityDecision::Allow => "Ausgewählte Apps erlauben",
    }
}
fn load(ctx: &egui::Context, shared: &Shared, device: String, key: String) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.fresh = false;
        state.error = None;
        state.info = None;
    }
    let (slot, context) = (shared.clone(), ctx.clone());
    let failed = shared.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("local-app-grants".into())
        .spawn(move || {
            let result = crate::daemon::apps_local_grants(&device, &key);
            let mut state = slot.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy = false;
            match result {
                Ok(snapshot) => install(&mut state, snapshot),
                Err(error) => state.error = Some(error),
            }
            context.request_repaint();
        })
    {
        let mut state = failed.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
        state.error = Some(error.to_string());
    }
}
fn save(
    ctx: &egui::Context,
    shared: &Shared,
    device: String,
    key: String,
    change: LocalAppGrantChange,
) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.error = None;
        state.info = None;
    }
    let (slot, context) = (shared.clone(), ctx.clone());
    let failed = shared.clone();
    if let Err(error)=std::thread::Builder::new().name("save-local-app-grant".into()).spawn(move|| {
        let result=crate::daemon::apps_set_local_grant(change).and_then(|result| {
            if !result.persisted||!result.applied||result.error.is_some() {return Err(format!("Appfreigabe nicht vollständig übernommen: gespeichert={}, angewendet={}, Grund={}",result.persisted,result.applied,result.error.as_deref().unwrap_or("Änderung bleibt im bestehenden Rechtejournal ausstehend")));}
            crate::daemon::apps_local_grants(&device,&key)
        });
        let mut state=slot.lock().unwrap_or_else(PoisonError::into_inner);state.busy=false;
        match result {Ok(snapshot)=>{install(&mut state,snapshot);state.info=Some("Appstartfreigabe dieser Quelle gespeichert. Befehls-, Terminal-, Bildschirm-, Anmelde- und TCP-Rechte bleiben erhalten.".into());},Err(error)=>{state.fresh=false;state.error=Some(error)}}context.request_repaint();
    }) {let mut state=failed.lock().unwrap_or_else(PoisonError::into_inner);state.busy=false;state.error=Some(error.to_string());}
}
