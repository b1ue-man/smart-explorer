//! The GUI panel of saved terminal/MCP commands (`se cp`/`se mv` jobs): the
//! explorer window shows it in its recovery section after the programs
//! registered it (`register_gui_panels`); rebinding is explicit as for the
//! window's own saved jobs.
use crate::transfer::{Endpoint, StoredEndpoint};
use eframe::egui;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct CommandState {
    catalog: Option<crate::cli::SavedCommandsCatalog>,
    running: std::collections::BTreeMap<String, crate::cli::RecoveredCommandTask>,
    error: Option<String>,
}

pub(crate) fn render_commands(ui: &mut egui::Ui, ctx: &egui::Context, places: &[(String, Endpoint)]) {
    static STATE: OnceLock<Mutex<CommandState>> = OnceLock::new();
    let mut state = STATE
        .get_or_init(|| Mutex::new(CommandState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.catalog.is_none() {
        match crate::cli::SavedCommandsCatalog::start() {
            Ok(catalog) => state.catalog = Some(catalog),
            Err(error) => state.error = Some(error),
        }
    }
    ui.separator();
    ui.label("Gespeicherte Terminal-/MCP-Aufträge");
    if ui.button("Terminalaufträge neu einlesen").clicked() {
        match crate::cli::SavedCommandsCatalog::start() {
            Ok(catalog) => state.catalog = Some(catalog),
            Err(error) => state.error = Some(error),
        }
    }
    if let Some(error) = &state.error {
        ui.label(error);
    }
    let Some(catalog) = &state.catalog else {
        return;
    };
    let snapshot = catalog.snapshot();
    let visible_ids: HashSet<String> = snapshot
        .records
        .iter()
        .map(|record| record.id.clone())
        .collect();
    if !snapshot.ready {
        ui.spinner();
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    if let Some(error) = &snapshot.error {
        ui.label(format!("Terminalaufträge unvollständig: {error}"));
    }
    for record in snapshot.records {
        ui.separator();
        ui.label(format!("Job-ID: {}", record.id));
        ui.label(format!("{} → {}", record.source_path, record.destination));
        if let Some(task) = state.running.get(&record.id) {
            if let Some(result) = task.result() {
                ui.label(match result {
                    Ok(()) => "Auftrag abgeschlossen".to_string(),
                    Err(error) => format!("Auftrag bleibt unbestätigt: {error}"),
                });
            } else {
                ui.label(task.progress_text());
                if ui.button("Wiederaufnahme abbrechen").clicked() {
                    task.cancel();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            continue;
        }
        let source_identity = if record.source.local {
            StoredEndpoint::of(&Endpoint::Local)
        } else {
            record.source.clone()
        };
        let target_identity = if record.target.local {
            StoredEndpoint::of(&Endpoint::Local)
        } else {
            record.target.clone()
        };
        let source = crate::app::choose_recovery_endpoint(ui, &record.id, "Quelle", &source_identity, places);
        let target = crate::app::choose_recovery_endpoint(ui, &record.id, "Ziel", &target_identity, places);
        if ui
            .add_enabled(
                source.is_some() && target.is_some(),
                egui::Button::new("Terminalauftrag wiederaufnehmen"),
            )
            .clicked()
        {
            if let (Some(source), Some(target)) = (source, target) {
                let id = record.id.clone();
                match crate::cli::RecoveredCommandTask::start(record, source, target) {
                    Ok(task) => {
                        state.running.insert(id, task);
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        }
    }
    for (id, task) in state
        .running
        .iter()
        .filter(|(id, _)| !visible_ids.contains(*id))
    {
        ui.label(format!("Job-ID: {id}"));
        if task.result().is_none() {
            ui.label("Wiederaufnahme läuft; Quellen-/Zielnamen bleiben bis zur frischen Bestätigung verborgen.");
            if ui.button("Diesen Auftrag abbrechen").clicked() {
                task.cancel();
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            ui.label("Auftrag beendet; Details erst nach bestätigtem Neueinlesen anzeigen.");
        }
    }
}
