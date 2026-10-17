//! „Exec-Protokoll“ of this host on the page „Befehle“: the entries of
//! `exec-audit.log` and `exec-audit-rejected.log` as `se share exec log`
//! shows them, read on request in a background thread, newest first, every
//! field cut and cleaned (`crate::share::exec_audit_entries`). Every entry
//! names its kind (Befehle or Bildschirm), the account and the confirmation
//! (PRM-M4); confirmations and ended grants are entries of their own.
use crate::app::theme;
use eframe::egui::{self, RichText};
use std::sync::{Arc, Mutex};

/// Choices for the number of newest entries (the reader caps at
/// `crate::share::MAX_EXEC_AUDIT_VIEW`).
const LINE_CHOICES: [usize; 3] = [50, 200, crate::share::MAX_EXEC_AUDIT_VIEW];
/// Entries that stand out: refusals and requests that ended without a yes.
const REFUSALS: [&str; 9] = [
    "auth_rejected",
    "penalty_refused",
    "start_refused",
    "unreadable",
    "approval_denied",
    "approval_cancelled",
    "approval_unavailable",
    "grant_expired",
    "helper_refused",
];
/// The kind filter: all entries, Exec entries, screen entries.
const KINDS: [(Option<&str>, &str); 3] = [
    (None, "Alle Arten"),
    (Some("exec"), "Befehle"),
    (Some("screen"), "Bildschirm"),
];

#[derive(Default)]
struct LogState {
    entries: Vec<crate::share::ExecAuditEntry>,
    loading: bool,
    loaded: bool,
    error: Option<String>,
}

pub(super) fn ui_exec_log(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("EXEC-PROTOKOLL DIESES HOSTS")
        .id_salt("share_exec_audit_log")
        .default_open(false)
        .show(ui, body);
}

fn body(ui: &mut egui::Ui) {
    ui.add(
        egui::Label::new(
            "Abgelehnte Anmeldungen, abgelehnte Starts, Starts und Enden, Rückfragen und ihre Antworten, abgelaufene Rechte, mit Geraet, Fingerprint, Art, Recht, Programm, Konto, Rückfrage und Ergebnis. Kein Text von Shell-Befehlen; Text des anderen Geraets ist auf 256 Zeichen gekuerzt. Dateien exec-audit.log und exec-audit-rejected.log im App-Datenordner (je 4 bis 8 MiB). Terminal: se share exec log.",
        )
        .wrap(),
    );
    let state = state(ui);
    let lines_id = egui::Id::new("share_exec_audit_lines");
    let mut lines = ui.data_mut(|data| data.get_temp::<usize>(lines_id).unwrap_or(LINE_CHOICES[0]));
    let (loading, loaded, error, entries) = state
        .lock()
        .map(|state| {
            (
                state.loading,
                state.loaded,
                state.error.clone(),
                state.entries.clone(),
            )
        })
        .unwrap_or_default();
    let mut reload = !loaded && !loading;
    ui.horizontal_wrapped(|ui| {
        ui.label("Neueste Eintraege:");
        for choice in LINE_CHOICES {
            if ui
                .selectable_label(lines == choice, choice.to_string())
                .clicked()
                && lines != choice
            {
                lines = choice;
                ui.data_mut(|data| data.insert_temp(lines_id, lines));
                reload = true;
            }
        }
        if ui
            .add_enabled(!loading, egui::Button::new("Neu laden"))
            .clicked()
        {
            reload = true;
        }
        if loading {
            ui.spinner();
        }
    });
    if reload {
        start_load(ui.ctx().clone(), state.clone(), lines);
    }
    if let Some(error) = error {
        ui.colored_label(
            theme::danger(ui),
            format!("Protokoll nicht lesbar: {error}"),
        );
    }
    let (filter, kind) = ui
        .horizontal_wrapped(|ui| (device_filter(ui, &entries), kind_filter(ui)))
        .inner;
    let shown: Vec<&crate::share::ExecAuditEntry> = entries
        .iter()
        .filter(|entry| filter.as_deref().is_none_or(|id| entry.device_id == id))
        .filter(|entry| kind.is_none_or(|kind| entry.kind == kind))
        .collect();
    if loaded && shown.is_empty() {
        ui.label("Keine Eintraege auf diesem Host.");
    }
    egui::ScrollArea::vertical()
        .id_salt("share_exec_audit_scroll")
        .max_height(360.0)
        .show(ui, |ui| {
            for entry in shown {
                entry_row(ui, entry);
            }
        });
}

fn entry_row(ui: &mut egui::Ui, entry: &crate::share::ExecAuditEntry) {
    let color = if REFUSALS.contains(&entry.event.as_str()) {
        theme::warning(ui)
    } else {
        theme::muted(ui)
    };
    ui.horizontal_wrapped(|ui| {
        ui.monospace(local_time(&entry.at));
        ui.colored_label(color, RichText::new(&entry.title).strong());
        ui.label(&entry.device);
    });
    if !entry.detail.is_empty() {
        ui.add(egui::Label::new(RichText::new(&entry.detail).small()).wrap());
    }
    ui.add_space(2.0);
}

/// „Alle Geraete“ or one device id seen in the loaded entries.
fn device_filter(ui: &mut egui::Ui, entries: &[crate::share::ExecAuditEntry]) -> Option<String> {
    let id = egui::Id::new("share_exec_audit_filter");
    let mut selected = ui.data_mut(|data| data.get_temp::<Option<String>>(id).unwrap_or(None));
    let mut devices: Vec<(&str, &str)> = Vec::new();
    for entry in entries {
        if !entry.device_id.is_empty() && !devices.iter().any(|(id, _)| *id == entry.device_id) {
            devices.push((entry.device_id.as_str(), entry.device.as_str()));
        }
    }
    if devices.is_empty() {
        return None;
    }
    let label = selected
        .as_deref()
        .and_then(|selected| devices.iter().find(|(id, _)| *id == selected))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| "Alle Geraete".into());
    egui::ComboBox::from_id_salt("share_exec_audit_device")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected, None, "Alle Geraete");
            for (device_id, name) in &devices {
                ui.selectable_value(&mut selected, Some(device_id.to_string()), *name);
            }
        });
    ui.data_mut(|data| data.insert_temp(id, selected.clone()));
    selected
}

/// „Alle Arten“, Befehle (`exec`) or Bildschirm (`screen`).
fn kind_filter(ui: &mut egui::Ui) -> Option<&'static str> {
    let id = egui::Id::new("share_exec_audit_kind");
    let mut selected = ui.data_mut(|data| data.get_temp::<usize>(id).unwrap_or(0));
    let (_, label) = KINDS[selected.min(KINDS.len() - 1)];
    egui::ComboBox::from_id_salt("share_exec_audit_kind_choice")
        .selected_text(label)
        .show_ui(ui, |ui| {
            for (index, (_, name)) in KINDS.iter().enumerate() {
                ui.selectable_value(&mut selected, index, *name);
            }
        });
    ui.data_mut(|data| data.insert_temp(id, selected));
    KINDS[selected.min(KINDS.len() - 1)].0
}

fn local_time(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| {
            if at.is_empty() {
                "-".into()
            } else {
                at.to_string()
            }
        })
}

fn state(ui: &mut egui::Ui) -> Arc<Mutex<LogState>> {
    let id = egui::Id::new("share_exec_audit_state");
    if let Some(state) = ui.data_mut(|data| data.get_temp::<Arc<Mutex<LogState>>>(id)) {
        return state;
    }
    let state = Arc::new(Mutex::new(LogState::default()));
    ui.data_mut(|data| data.insert_temp(id, state.clone()));
    state
}

fn start_load(ctx: egui::Context, state: Arc<Mutex<LogState>>, lines: usize) {
    match state.lock() {
        Ok(mut current) if !current.loading => current.loading = true,
        _ => return,
    }
    std::thread::spawn(move || {
        let result = crate::share::exec_audit_entries(lines);
        if let Ok(mut current) = state.lock() {
            current.loading = false;
            current.loaded = true;
            match result {
                Ok(entries) => {
                    current.entries = entries;
                    current.error = None;
                }
                Err(error) => current.error = Some(error.to_string()),
            }
        }
        ctx.request_repaint();
    });
}
