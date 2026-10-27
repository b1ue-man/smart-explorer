//! One device, every exact relationship. All editing retains its source key.
use crate::app::{device_rights_model as model, theme, App};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::access_relation_view::{AccessRelationView, DeviceAccessView};
use eframe::egui::{self, RichText};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    let should_load = {
        let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        let due = state
            .loaded
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(2));
        if due && !state.loading {
            state.loading = true;
            true
        } else {
            false
        }
    };
    if should_load {
        let (slot, ctx, device, key) = (
            shared.clone(),
            ui.ctx().clone(),
            device_id.to_string(),
            public_key.to_string(),
        );
        let start = std::thread::Builder::new()
            .name("device-rights".into())
            .spawn(move || {
                let result = crate::daemon::device_access_rights(&device, &key);
                let mut state = slot.lock().unwrap_or_else(|poison| poison.into_inner());
                match result {
                    Ok(view) => {
                        state.view = Some(view);
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
                state.loading = false;
                state.loaded = Some(Instant::now());
                ctx.request_repaint();
            });
        if let Err(error) = start {
            let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
            state.loading = false;
            state.error = Some(format!(
                "Rechteabfrage konnte nicht gestartet werden: {error}"
            ));
            state.loaded = Some(Instant::now());
        }
    }
    ui.ctx().request_repaint_after(Duration::from_secs(2));
    let (view, error, loading) = {
        let state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        (state.view.clone(), state.error.clone(), state.loading)
    };
    if loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Rechtequellen werden gelesen …");
        });
    }
    if let Some(error) = &error {
        ui.colored_label(theme::danger(ui), format!("Rechte nicht aktuell: {error}"));
    }
    let Some(view) = view else {
        return;
    };
    if model::remaining_room_access(&view) {
        ui.colored_label(theme::warning(ui), "Der Entzug einer Direktfreigabe beendet den Raumzugang nicht. Verbleibende Raumrechte stehen bei ihrer jeweiligen Quelle.");
    }
    if view.relations.is_empty() {
        ui.label("Für diese Geräteidentität gibt es keine Freigabe.");
    }
    // One relation at a time; a choice only when the device has several
    // (Direct and rooms). Without a choice the first active one is shown.
    let selected_id = id.with("selected_source");
    let stored = ui.data_mut(|data| data.get_temp::<String>(selected_id));
    let mut index = stored
        .and_then(|key| {
            view.relations
                .iter()
                .position(|relation| model::target_key(&relation.target) == key)
        })
        .or_else(|| view.relations.iter().position(|relation| relation.active))
        .unwrap_or(0);
    let names: Vec<String> = view.relations.iter().map(model::source_name).collect();
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    crate::app::device_page_layout::relation_picker(ui, &labels, &mut index);
    if let Some(source) = view.relations.get(index) {
        let key = model::target_key(&source.target);
        ui.data_mut(|data| data.insert_temp(selected_id, key.clone()));
        if !source.active {
            ui.colored_label(
                theme::warning(ui),
                "Diese Freigabe ist inaktiv – derzeit kein Zugang darüber.",
            );
        }
        source_table(ui, source);
        for line in model::condition_lines(source) {
            ui.add(egui::Label::new(RichText::new(line).small()).wrap());
        }
        ui.add_space(4.0);
        crate::app::share_exec_ui::ui_exec_card(app, ui, &key, true);
        if source.relation_kind == "room" {
            ui.horizontal_wrapped(|ui| {
                ui.small("Dateifreigaben und Schreibrecht eines Raums gelten für alle zugelassenen Mitglieder.");
                if ui.button("Raum öffnen …").clicked() {
                    app.open_share_room_page(&source.relation_id);
                }
            });
        }
    }
    egui::CollapsingHeader::new("Diagnose dieser Geräteidentität")
        .id_salt(id.with("diagnostics"))
        .show(ui, |ui| {
            ui.monospace(device_id);
            if let Ok(text) = serde_json::to_string_pretty(&view) {
                if ui.button("Diagnose kopieren").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                ui.add(egui::Label::new(RichText::new(text).monospace().small()).wrap());
            }
        });
}

fn source_table(ui: &mut egui::Ui, relation: &AccessRelationView) {
    let mut rows = vec![(
        "Dateien lesen".to_string(),
        if !model::has_files(relation) {
            "Keine Ordner oder Verbindungen freigegeben".into()
        } else {
            "Freigegebene Ordner und Verbindungen".into()
        },
    )];
    rows.push((
        "Dateien schreiben".into(),
        if relation.may_write {
            "Innerhalb der Schreibrechte jedes Ordners".into()
        } else {
            "Aus".into()
        },
    ));
    let facts = crate::share::access_catalog::HostFacts {
        graphical_session: true,
        account_helper: crate::share::access_catalog::HelperStatus {
            state: crate::share::access_catalog::HelperState::NotNeeded,
            version: None,
            detail: None,
        },
    };
    let mut capabilities: Vec<_> = crate::share::access_catalog::capability_catalog(&facts)
        .into_iter()
        .map(|entry| entry.capability)
        .collect();
    capabilities.extend(Capability::ALL.into_iter().filter(|capability| {
        capability.requires_context()
            && (relation.grant.capabilities.contains_key(capability)
                || relation.grant.constraints.has(*capability))
    }));
    for capability in capabilities {
        let decision = relation
            .grant
            .capabilities
            .get(&capability)
            .copied()
            .unwrap_or(CapabilityDecision::Off);
        let effective = if relation.grant.expired {
            "Abgelaufen"
        } else if !relation.active {
            "Inaktiv"
        } else if capability.requires_context()
            && !relation
                .grant
                .constraints
                .available_at(capability, crate::share::core_now_secs())
        {
            "Zielbedingungen nicht verfügbar oder abgelaufen"
        } else {
            model::decision_label(decision)
        };
        let chosen = model::decision_label(decision);
        rows.push((
            model::capability_label(capability).into(),
            if effective == chosen {
                chosen.to_string()
            } else {
                format!("{chosen} (wirksam: {effective})")
            },
        ));
    }
    egui::Grid::new("rights_rows")
        .num_columns(2)
        .striped(true)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (label, value) in &rows {
                ui.label(RichText::new(label).color(theme::muted(ui)));
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
    for connection in &relation.exports.shared_connections {
        let access =
            if connection.access == crate::share::ExportAccess::ReadWrite && relation.may_write {
                "Lesen und Schreiben"
            } else {
                "Nur lesen"
            };
        ui.add(
            egui::Label::new(format!(
                "Verbindung {}: {access}; Providerrechte gelten weiterhin",
                connection.account
            ))
            .wrap(),
        );
    }
    if relation.exports.include_connections {
        ui.small("Gespeicherte alte Verbindungsfreigabe vorhanden; siehe Freigaben dieses Geräts beziehungsweise Raums.");
    }

    for root in &relation.exports.roots {
        let access = if root.access == crate::share::ExportAccess::ReadWrite && relation.may_write {
            "Lesen und Schreiben"
        } else {
            "Nur lesen"
        };
        ui.label(format!("{}: {access}", root.label))
            .on_hover_text(&root.path);
    }
}
