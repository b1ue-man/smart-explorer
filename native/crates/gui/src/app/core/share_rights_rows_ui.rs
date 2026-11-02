//! The rows of the rights window (plan 2026-10-31 §F, F3): folders, write,
//! the four decisions, Remote Desktop and duration. Each row edits the
//! draft only; nothing is stored until „Bestätigen“.
use eframe::egui::{self, RichText};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::share_rights_model::{ExpiryChoice, FolderMode, RightsDraft, DECISION_ROWS};
use crate::app::theme;
use crate::share::access_grant::{Capability, CapabilityDecision};

/// The Direct folders the window offers, as (label, path).
pub(super) struct FolderInfo {
    pub(super) exported: Vec<(String, String)>,
    /// Saved connections are exported (`/Verbindungen`).
    pub(super) connections: bool,
}

impl FolderInfo {
    /// Exported folders followed by the ones picked in this window.
    fn all(&self, draft: &RightsDraft) -> Vec<(String, String)> {
        let mut all = self.exported.clone();
        for (label, path) in &draft.added_folders {
            if !all.iter().any(|(_, known)| known == path) {
                all.push((label.clone(), path.clone()));
            }
        }
        all
    }
}

pub(super) fn folders(ui: &mut egui::Ui, draft: &mut RightsDraft, info: &FolderInfo) {
    let all = info.all(draft);
    let paths: Vec<String> = all.iter().map(|(_, path)| path.clone()).collect();
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for (mode, label) in [
                (FolderMode::All, "Alle"),
                (FolderMode::None, "Keine"),
                (FolderMode::Selected, "Auswahl"),
            ] {
                if ui
                    .selectable_label(draft.folder_mode() == mode, label)
                    .clicked()
                {
                    draft.set_folder_mode(mode, &paths);
                }
            }
        });
        match draft.folder_mode() {
            FolderMode::All if all.is_empty() => {
                ui.label(
                    RichText::new("Noch kein Ordner freigegeben.")
                        .small()
                        .color(theme::muted(ui)),
                );
            }
            FolderMode::All => {
                // G5 (review 2026-10-31): a room takes a copy of the Direct
                // folders of now (`connection_rights_rooms.rs`).
                let text = if draft.is_room() {
                    "Die Direkt-Ordner von jetzt; später hinzugefügte kommen nicht von selbst in den Raum."
                } else {
                    "Alle Direkt-Ordner, auch später hinzugefügte."
                };
                ui.label(RichText::new(text).small().color(theme::muted(ui)));
            }
            FolderMode::None => {}
            FolderMode::Selected => {
                for (label, path) in &all {
                    let mut on = draft.folder_ticked(path);
                    let text = if label.is_empty() || label == path {
                        path.clone()
                    } else {
                        format!("{label} · {path}")
                    };
                    if ui.checkbox(&mut on, text).changed() {
                        draft.tick_folder(path, on);
                    }
                }
                if info.connections {
                    let mut on = draft.connections_ticked();
                    if ui.checkbox(&mut on, "Gespeicherte Verbindungen").changed() {
                        draft.tick_connections(on);
                    }
                }
            }
        }
        if ui
            .button("Ordner hinzufügen …")
            .on_hover_text(
                "Der Ordner wird mit „Bestätigen“ Direkt-Freigabe und für dieses Gerät gewählt.",
            )
            .clicked()
        {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                let label = folder
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let path = folder.to_string_lossy().replace('\\', "/");
                draft.add_folder(&label, &path, &paths);
            }
        }
    });
}

pub(super) fn write(ui: &mut egui::Ui, draft: &mut RightsDraft) {
    ui.checkbox(
        &mut draft.rights.write,
        "Dateien ändern, löschen, Papierkorb",
    )
    .on_hover_text("Schreiben gilt in den Ordnern, die Schreiben erlauben.");
}

/// The four decisions, one row each.
pub(super) fn decisions(ui: &mut egui::Ui, draft: &mut RightsDraft) {
    for (capability, label, hint) in DECISION_ROWS {
        ui.label(label).on_hover_text(hint);
        decision(ui, draft, capability);
        ui.end_row();
        if capability == Capability::ScreenControl
            && draft.rights.screen_control == CapabilityDecision::Allow
        {
            ui.label("Remote Desktop");
            rdp(ui);
            ui.end_row();
        }
    }
}

fn decision(ui: &mut egui::Ui, draft: &mut RightsDraft, capability: Capability) {
    let current = draft.rights.decision(capability);
    ui.horizontal(|ui| {
        for (value, label) in [
            (CapabilityDecision::Off, "Aus"),
            (CapabilityDecision::Ask, "Fragen"),
            (CapabilityDecision::Allow, "Erlauben"),
        ] {
            if ui.selectable_label(current == value, label).clicked() {
                draft.set_decision(capability, value);
            }
        }
    });
}

pub(super) fn expiry(ui: &mut egui::Ui, draft: &mut RightsDraft) {
    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            if let ExpiryChoice::Keep(_) = draft.expiry {
                // Shown as chosen; choosing it again changes nothing.
                let _ = ui.selectable_label(true, ExpiryChoice::Keep(0).label());
            }
            for choice in ExpiryChoice::FIXED {
                if ui
                    .selectable_label(draft.expiry == choice, choice.label())
                    .clicked()
                {
                    draft.expiry = choice;
                }
            }
        });
        ui.label(
            RichText::new("Gilt für Befehle, Terminal und Desktop, nicht für Dateien.")
                .small()
                .color(theme::muted(ui)),
        );
    });
}

/// The state of this device's Remote Desktop, read in the background.
#[derive(Default)]
struct RdpProbe {
    /// (ready, reason when it is not)
    result: Option<(bool, Option<String>)>,
    at: Option<Instant>,
    running: bool,
}

/// Re-read at most this often (the Windows probe starts PowerShell).
const PROBE_EVERY: Duration = Duration::from_secs(60);

/// „Remote Desktop“ row: the background service sets it up by itself as soon
/// as a device with „Desktop steuern: Erlauben“ is present (rule 2026-10-28,
/// `rdp::host_setup::peer_present`), so the row only says what will happen
/// or that it is ready.
pub(super) fn rdp(ui: &mut egui::Ui) {
    let id = egui::Id::new("share_rights_rdp_probe");
    let shared: Arc<Mutex<RdpProbe>> = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(RdpProbe::default())))
            .clone()
    });
    let due = {
        let mut probe = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let due = !probe.running && probe.at.is_none_or(|at| at.elapsed() >= PROBE_EVERY);
        probe.running |= due;
        due
    };
    if due {
        let (state, ctx) = (shared.clone(), ui.ctx().clone());
        let spawned = std::thread::Builder::new()
            .name("rights-rdp-probe".into())
            .spawn(move || {
                let probe = crate::rdp::probe_local_host();
                let ready = probe.preferred_service().is_some();
                let mut slot = state.lock().unwrap_or_else(PoisonError::into_inner);
                slot.result = Some((ready, probe.unavailable_reason.clone()));
                slot.at = Some(Instant::now());
                slot.running = false;
                ctx.request_repaint();
            });
        if spawned.is_err() {
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .running = false;
        }
    }
    let result = shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .result
        .clone();
    match result {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Wird geprüft …");
            });
        }
        Some((true, _)) => {
            ui.label("Bereit.");
        }
        Some((false, reason)) => {
            ui.vertical(|ui| {
                ui.label(
                    "Richtet sich von selbst ein, sobald das Gerät erreichbar ist (Windows: einmal Administratorfreigabe, Linux: polkit).",
                );
                if let Some(reason) = reason {
                    ui.label(RichText::new(reason).small().color(theme::muted(ui)));
                }
            });
        }
    }
}
