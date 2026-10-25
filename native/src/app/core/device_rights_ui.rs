//! One device, every exact relationship. All editing retains its source key.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use eframe::egui::{self, RichText};
use crate::app::{device_rights_model as model, theme, App};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::access_relation_view::{AccessRelationView, DeviceAccessView};

#[derive(Default)]
struct Snapshot {
    view: Option<DeviceAccessView>,
    error: Option<String>,
    loading: bool,
    loaded: Option<Instant>,
}
type Shared = Arc<Mutex<Snapshot>>;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui, device_id: &str, public_key: &str) {
    let id = egui::Id::new(("device_rights", device_id, public_key));
    let shared = ui.data_mut(|data| data.get_temp_mut_or_default::<Shared>(id).clone());
    let refresh = ui.button("Rechte aktualisieren").clicked();
    let should_load = {
        let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        let due = refresh || state.loaded.is_none_or(|at| at.elapsed() >= Duration::from_secs(2));
        if due && !state.loading { state.loading = true; true } else { false }
    };
    if should_load {
        let (slot, ctx, device, key) = (shared.clone(), ui.ctx().clone(), device_id.to_string(), public_key.to_string());
        let start = std::thread::Builder::new().name("device-rights".into()).spawn(move || {
            let result = crate::daemon::access_ipc::device_access_rights(&device, &key);
            let mut state = slot.lock().unwrap_or_else(|poison| poison.into_inner());
            match result { Ok(view) => { state.view = Some(view); state.error = None; }, Err(error) => state.error = Some(error) }
            state.loading = false; state.loaded = Some(Instant::now()); ctx.request_repaint();
        });
        if let Err(error) = start {
            let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
            state.loading = false; state.error = Some(format!("Rechteabfrage konnte nicht gestartet werden: {error}")); state.loaded = Some(Instant::now());
        }
    }
    ui.ctx().request_repaint_after(Duration::from_secs(2));
    let (view, error, loading) = {
        let state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        (state.view.clone(), state.error.clone(), state.loading)
    };
    if loading { ui.horizontal(|ui| { ui.spinner(); ui.label("Rechtequellen werden gelesen …"); }); }
    if let Some(error) = &error { ui.colored_label(theme::danger(ui), format!("Rechte nicht aktuell: {error}")); }
    let Some(view) = view else { return; };
    ui.label("Jede Verbindung verwendet genau ihre eigene Freigabe. Der Gesamtzugang zeigt erreichbare Beziehungen mit deren Bedingungen.");
    if model::remaining_room_access(&view) {
        ui.colored_label(theme::warning(ui), "Der Entzug einer Direktfreigabe beendet den Raumzugang nicht. Verbleibende Raumrechte stehen bei ihrer jeweiligen Quelle.");
    }
    theme::section(ui, "Rechte je Beziehung");
    if view.relations.is_empty() { ui.label("Für diese gepinnte Geräteidentität gibt es keine Freigabequelle."); }
    let selected_id = id.with("selected_source");
    let mut selected = ui.data_mut(|data| data.get_temp::<String>(selected_id));
    for relation in &view.relations {
        ui.push_id(model::target_key(&relation.target), |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(model::source_name(relation)).strong());
                ui.label(if relation.active { "Beziehung aktiv" } else { "Beziehung inaktiv – derzeit kein Zugang über diese Quelle" });
                source_table(ui, relation);
                for line in model::condition_lines(relation) { ui.add(egui::Label::new(line).wrap()); }
                if ui.add_enabled(error.is_none(), egui::Button::new("Diese Quelle bearbeiten …")).clicked() {
                    selected = Some(model::target_key(&relation.target));
                }
            });
        });
    }
    theme::section(ui, "Erreichbarer Gesamtzugang");
    let active_files = view.relations.iter().filter(|relation| relation.active && model::has_files(relation));
    let mut any = false;
    for relation in active_files { any = true; ui.label(format!("Dateien und Verbindungen über {} – siehe die freigegebenen Bereiche und Schreibgrenzen oben", model::source_name(relation))); }
    for right in &view.reachable {
        any = true;
        let source = view.relations.iter().find(|relation| relation.target == right.source);
        if let Some(source) = source {
            ui.label(format!("{}: {} über {}", model::capability_label(right.capability), model::decision_label(right.decision), model::source_name(source)));
        }
    }
    if !any { ui.label("Derzeit kein Zugang über eine aktive Beziehung."); }
    if let Some(key) = selected.clone() {
        if let Some(source) = view.relations.iter().find(|relation| model::target_key(&relation.target) == key) {
            theme::section(ui, &format!("Bearbeiten: {}", model::source_name(source)));
            if error.is_none() { crate::app::share_exec_ui::ui_exec_target(app, ui, &key); }
            if source.relation_kind == "room" {
                ui.small("Dateifreigaben und das Schreibrecht dieses Raums gelten für alle zugelassenen Mitglieder. Sie werden auf der Raumseite bearbeitet.");
                if ui.button("Raumrichtlinie öffnen …").clicked() { app.open_share_room_page(&source.relation_id); }
            }
            if ui.button("Bearbeitung schließen").clicked() { selected = None; }
        } else { selected = None; }
    }
    ui.data_mut(|data| { if let Some(selected) = selected { data.insert_temp(selected_id, selected); } else { data.remove::<String>(selected_id); } });
    egui::CollapsingHeader::new("Diagnose dieser Geräteidentität").id_salt(id.with("diagnostics")).show(ui, |ui| {
        ui.monospace(device_id);
        if let Ok(text) = serde_json::to_string_pretty(&view) {
            if ui.button("Diagnose kopieren").clicked() { ui.ctx().copy_text(text.clone()); }
            ui.add(egui::Label::new(RichText::new(text).monospace().small()).wrap());
        }
    });
}

fn source_table(ui: &mut egui::Ui, relation: &AccessRelationView) {
    let mut rows = vec![("Dateien lesen".to_string(), if !model::has_files(relation) { "Keine Ordner oder Verbindungen freigegeben".into() } else { "Freigegebene Ordner und Verbindungen".into() })];
    rows.push(("Dateien schreiben".into(), if relation.may_write { "Innerhalb der Schreibrechte jedes Ordners".into() } else { "Aus".into() }));
    let facts = crate::share::access_catalog::HostFacts {
        graphical_session: true,
        account_helper: crate::share::access_catalog::HelperStatus { state: crate::share::access_catalog::HelperState::NotNeeded, version: None, detail: None },
    };
    let mut capabilities: Vec<_> = crate::share::access_catalog::capability_catalog(&facts).into_iter().map(|entry| entry.capability).collect();
    capabilities.extend(Capability::ALL.into_iter().filter(|capability| capability.requires_context() && (relation.grant.capabilities.contains_key(capability) || relation.grant.constraints.has(*capability))));
    for capability in capabilities {
        let decision = relation.grant.capabilities.get(&capability).copied().unwrap_or(CapabilityDecision::Off);
        let effective = if relation.grant.expired { "Abgelaufen" } else if !relation.active { "Inaktiv" }
            else if capability.requires_context() && !relation.grant.constraints.available_at(capability, crate::share::core_now_secs()) { "Zielbedingungen nicht verfügbar oder abgelaufen" }
            else { model::decision_label(decision) };
        rows.push((model::capability_label(capability).into(), format!("{} · wirksam: {effective}", model::decision_label(decision))));
    }
    let labels: Vec<_> = rows.iter().map(|(label, _)| label.as_str()).collect();
    let mut form = crate::app::settings_form::Form::new(ui, "rights_rows", &labels);
    for (label, value) in &rows { form.row(ui, label, |ui| { ui.add(egui::Label::new(value).wrap()); }); }
    for connection in &relation.exports.shared_connections {
        let access = if connection.access == crate::share::ExportAccess::ReadWrite && relation.may_write { "Lesen und Schreiben" } else { "Nur lesen" };
        ui.add(egui::Label::new(format!("Verbindung {}: {access}; Providerrechte gelten weiterhin", connection.account)).wrap());
    }
    if relation.exports.include_connections { ui.small("Gespeicherte alte Verbindungsfreigabe vorhanden; siehe Freigaben dieses Geräts beziehungsweise Raums."); }

    for root in &relation.exports.roots {
        let access = if root.access == crate::share::ExportAccess::ReadWrite && relation.may_write { "Lesen und Schreiben" } else { "Nur lesen" };
        ui.label(format!("{}: {access}", root.label)).on_hover_text(&root.path);
    }
}
