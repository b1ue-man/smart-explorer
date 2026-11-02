//! Rebind saved jobs only to explicitly selected, identically scoped endpoints.
use crate::transfer::{Endpoint, JobRecord, RecoveryCatalog, StoredEndpoint, TransferJob};
use eframe::egui;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// The panel of saved terminal/MCP commands. The CLI crate owns those commands
/// and builds on this crate, so the programs register its panel before the
/// window opens (plan docs/plaene/2026-11-01-idealzustand, D13).
pub type CommandsPanel = fn(&mut egui::Ui, &egui::Context, &[(String, Endpoint)]);

static COMMANDS_PANEL: OnceLock<CommandsPanel> = OnceLock::new();

/// Registers the panel of saved terminal/MCP commands; the first registration
/// stays.
pub fn register_commands_panel(panel: CommandsPanel) {
    let _ = COMMANDS_PANEL.set(panel);
}

fn catalog() -> &'static Mutex<Option<RecoveryCatalog>> {
    static CATALOG: OnceLock<Mutex<Option<RecoveryCatalog>>> = OnceLock::new();
    CATALOG.get_or_init(|| Mutex::new(None))
}

pub(in crate::app) fn render(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    places: &[(String, Endpoint)],
    known: &HashSet<String>,
) -> Option<Result<TransferJob, String>> {
    let mut catalog = catalog()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if catalog.is_none() {
        match RecoveryCatalog::start() {
            Ok(value) => *catalog = Some(value),
            Err(error) => {
                ui.label(format!("Gespeicherte Aufträge laden: {error}"));
                return None;
            }
        }
    }
    let state = catalog.as_ref()?.snapshot();
    let mut outcome = None;
    ui.collapsing("Gespeicherte unbestätigte Aufträge", |ui| {
        if ui.button("Aufträge neu einlesen").clicked() {
            match RecoveryCatalog::start() { Ok(value) => *catalog = Some(value), Err(error) => { ui.label(error.to_string()); } }
        }
        if !state.ready { ui.spinner(); ui.label("Gespeicherte Jobkennungen werden gelesen…"); ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
        if let Some(error) = &state.error { ui.label(format!("Aufträge konnten nicht vollständig gelesen werden: {error}")); }
        let records: Vec<&JobRecord> = state.records.iter().filter(|record| !known.contains(&record.id)).collect();
        if state.ready && records.is_empty() && state.error.is_none() { ui.label("Keine zusätzlichen unbestätigten Aufträge gespeichert."); }
        for record in records {
            ui.separator();
            ui.label(format!("Job-ID: {}", record.id));
            ui.label(format!("Zielroot: {}", record.target_dir));
            ui.collapsing(format!("{} gespeicherte Quellen", record.items.len()), |ui| {
                match &record.items {
                    crate::transfer::JobItems::Roots { paths, .. } => for path in paths { ui.label(path); },
                    crate::transfer::JobItems::Pairs(pairs) => for pair in pairs { ui.label(format!("{} → {}", pair.source, pair.rel)); },
                }
            });
            ui.label("Quelle und Ziel ausdrücklich wählen. Verbindung, Root, Quellbeleg und gespeicherte Entscheidungen werden vor dem Start bestätigt.");
            let source = choose(ui, &record.id, "Quelle", &record.source, places);
            let target = choose(ui, &record.id, "Ziel", &record.target, places);
            if ui.add_enabled(source.is_some() && target.is_some(), egui::Button::new("Diesen Auftrag wiederaufnehmen")).clicked() {
                if let (Some(source), Some(target)) = (source, target) { outcome = Some(record.bind(source, target)); }
            }
        }
        if let Some(panel) = COMMANDS_PANEL.get() {
            panel(ui, ctx, places);
        }
    });
    outcome
}

pub fn choose(
    ui: &mut egui::Ui,
    record_id: &str,
    side: &str,
    expected: &StoredEndpoint,
    places: &[(String, Endpoint)],
) -> Option<Endpoint> {
    let id = egui::Id::new(("recovery-binding", record_id, side));
    let mut selected = ui.data(|data| data.get_temp::<usize>(id));
    let available: Vec<usize> = places
        .iter()
        .enumerate()
        .filter(|(_, (_, endpoint))| StoredEndpoint::of(endpoint) == *expected)
        .map(|(index, _)| index)
        .collect();
    if !selected.is_some_and(|index| available.contains(&index)) {
        selected = None;
    }
    egui::ComboBox::from_id_source(id)
        .selected_text(
            selected
                .map(|index| places[index].0.as_str())
                .unwrap_or("Verbindung ausdrücklich wählen…"),
        )
        .show_ui(ui, |ui| {
            for index in &available {
                ui.selectable_value(&mut selected, Some(*index), &places[*index].0);
            }
        });
    if available.is_empty() {
        ui.label(format!(
            "{side}: die gespeicherte Verbindung muss zuerst im Explorer geöffnet werden."
        ));
    }
    if let Some(index) = selected {
        ui.data_mut(|data| data.insert_temp(id, index));
    }
    selected.map(|index| places[index].1.clone())
}

