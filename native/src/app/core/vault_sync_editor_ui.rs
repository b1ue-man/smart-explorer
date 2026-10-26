//! Vault sync jobs in the sync windows (U29-9, T15, T17): the dialog
//! „Synchronisieren…“ that creates a job without a folder setup, and the
//! pages of the job editor for a vault job. The fixed values of a vault job
//! (C5.6) are shown grey with the hover „Bei Tresor-Jobs fest.“; only the
//! name, the direction, the deletion policy of a one-sided job, „Aktiv“ and
//! the schedule can be changed. The sides are fixed once the job exists.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_feed::{self, guard, poke, poll, spawn, Poll};
use super::vault_models::VaultRow;
use super::vault_models_sync::{
    broken_vault_job_lines, delete_label, direction_label, editor_model_for, normalize,
    same_vault_sites, CreateDraft, FieldId, TargetChoice, FIXED_HOVER, ONE_SIDED_DELETE,
};
use super::vault_sidebar_ui::{Effect, PickTarget};
use crate::bisync::Direction;
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::SyncJob;

const STORE_PREFIX: &str = "vault-store://";
/// How often the files of the vault jobs are read again for the list of broken ones.
const BROKEN_EVERY: Duration = Duration::from_secs(3);

struct CreateDialog {
    origin: String,
    draft: CreateDraft,
    rx: Option<Receiver<Result<String, String>>>,
    error: Option<String>,
}

fn state() -> &'static Mutex<Option<CreateDialog>> {
    static STATE: OnceLock<Mutex<Option<CreateDialog>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// Opens the dialog „Synchronisieren…“ for the site `origin` (side A).
pub(crate) fn open_create(origin: &str) {
    let label = vault_feed::rows()
        .iter()
        .find(|row| row.site == origin)
        .map_or_else(|| origin.to_string(), |row| row.label.clone());
    *guard(state()) = Some(CreateDialog {
        origin: origin.to_string(),
        draft: CreateDraft::new(&label),
        rx: None,
        error: None,
    });
}

/// A folder picked for „Ordner …“.
pub(crate) fn picked(folder: String) {
    if let Some(dialog) = guard(state()).as_mut() {
        dialog.draft.folder = folder;
        dialog.draft.target = TargetChoice::Folder;
    }
}

/// Creates the job (`vault::create_vault_job`): the second side, then the job
/// with the direction and the name of the draft; the checks of the editor and
/// of the fixed vault values run when the job is built.
fn create(origin: &str, draft: &CreateDraft) -> Result<String, String> {
    let store = crate::vault::RegistryStore::open_default();
    let folder = draft.folder.trim().to_string();
    let target = match &draft.target {
        TargetChoice::Replica => crate::vault::CopyTarget::Replica,
        TargetChoice::Site(site) => crate::vault::CopyTarget::Site(site),
        TargetChoice::Folder => crate::vault::CopyTarget::Location(&folder),
    };
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    let replicas = crate::vault::default_replicas_root();
    let taken = |id: &str| crate::syncjobs::job_id_taken(id);
    let save = |job: &SyncJob| -> Result<(), String> {
        let mut editor = JobEditor::from_job(job);
        editor.name = draft.name.clone();
        editor.direction = draft.direction;
        editor.delete_policy = normalize(draft.direction, draft.delete_policy);
        editor.move_files = false;
        let built = editor.build_sync_job(Some(job))?;
        crate::syncjobs::upsert(&built).map_err(|error| error.to_string())
    };
    let place = crate::vault::JobPlace {
        replicas_root: &replicas,
        resolve: &resolve,
        id_taken: &taken,
        save: &save,
    };
    crate::vault::create_vault_job(&store, origin, target, Some(&draft.name), &place)
        .map(|created| created.job.name)
}

/// Draws the dialog and collects the answer of the thread.
pub(crate) fn show(ctx: &egui::Context) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut held = guard(state());
    let Some(dialog) = held.as_mut() else {
        return effects;
    };
    let mut finished = false;
    match poll(&mut dialog.rx) {
        Poll::Ready(Ok(name)) => {
            effects.push(Effect::Notice(format!(
                "Tresor-Sync „{name}“ angelegt. Der erste Lauf füllt die zweite Seite."
            )));
            effects.push(Effect::ReloadJobs);
            poke();
            finished = true;
        }
        Poll::Ready(Err(error)) => dialog.error = Some(error),
        Poll::Gone => dialog.error = Some("Das Anlegen endete ohne Antwort.".to_string()),
        Poll::Pending => {}
    }
    if finished {
        *held = None;
        return effects;
    }
    let rows = vault_feed::rows();
    let Some(origin) = rows.iter().find(|row| row.site == dialog.origin).cloned() else {
        *held = None;
        return effects;
    };
    let busy = dialog.rx.is_some();
    let (mut create_now, mut cancel) = (false, false);
    crate::app::dialog_geometry::window(
        ctx,
        "vault_sync_editor_ui.0",
        format!("Synchronisieren – {}", origin.label),
    )
    .id(egui::Id::new("vault_sync_create"))
    .collapsible(false)
    .resizable(true)
    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
    .show(ctx, |ui| {
        create_form(ui, dialog, &origin, &rows, busy, &mut effects);
        ui.separator();
        ui.horizontal(|ui| {
            let ready = dialog.draft.issue().is_none() && !busy;
            if ui
                .add_enabled(ready, egui::Button::new("Anlegen"))
                .clicked()
            {
                create_now = true;
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Abbrechen"))
                .clicked()
            {
                cancel = true;
            }
            if busy {
                ui.spinner();
            }
        });
    });
    if cancel {
        *held = None;
    } else if create_now {
        dialog.error = None;
        let (origin_site, draft) = (dialog.origin.clone(), dialog.draft.clone());
        match spawn(ctx, "vault-sync-create", move || {
            create(&origin_site, &draft)
        }) {
            Ok(rx) => dialog.rx = Some(rx),
            Err(error) => dialog.error = Some(error),
        }
    }
    effects
}

fn create_form(
    ui: &mut egui::Ui,
    dialog: &mut CreateDialog,
    origin: &VaultRow,
    rows: &[VaultRow],
    busy: bool,
    effects: &mut Vec<Effect>,
) {
    let others = same_vault_sites(rows, origin);
    egui::Grid::new("vault_sync_create_grid")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("Ort A");
            ui.add_enabled(false, egui::Label::new(origin.label.as_str()))
                .on_disabled_hover_text("Der Tresor, von dem aus du synchronisierst.");
            ui.end_row();
            ui.label("Name");
            ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut dialog.draft.name).desired_width(280.0),
            );
            ui.end_row();
            ui.label("Ort B");
            ui.vertical(|ui| {
                ui.radio_value(
                    &mut dialog.draft.target,
                    TargetChoice::Replica,
                    "Automatische Kopie",
                );
                let mut chosen = match &dialog.draft.target {
                    TargetChoice::Site(site) => site.clone(),
                    _ => others
                        .first()
                        .map(|row| row.site.clone())
                        .unwrap_or_default(),
                };
                ui.add_enabled_ui(!others.is_empty(), |ui| {
                    let selected = matches!(dialog.draft.target, TargetChoice::Site(_));
                    if ui.radio(selected, "Anderer Tresorort …").clicked() {
                        dialog.draft.target = TargetChoice::Site(chosen.clone());
                    }
                });
                if matches!(dialog.draft.target, TargetChoice::Site(_)) {
                    let shown = others
                        .iter()
                        .find(|row| row.site == chosen)
                        .map_or("–", |row| row.label.as_str());
                    egui::ComboBox::from_id_salt("vault_sync_other_site")
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            for row in &others {
                                ui.selectable_value(&mut chosen, row.site.clone(), &row.label);
                            }
                        });
                    dialog.draft.target = TargetChoice::Site(chosen);
                }
                ui.radio_value(&mut dialog.draft.target, TargetChoice::Folder, "Ordner …");
                if dialog.draft.target == TargetChoice::Folder {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut dialog.draft.folder)
                                .hint_text("neuer oder leerer Ordner")
                                .desired_width(220.0),
                        );
                        if ui.button("Wählen …").clicked() {
                            effects.push(Effect::PickFolder(PickTarget::SyncFolder));
                        }
                    });
                }
            });
            ui.end_row();
            ui.label("Richtung");
            direction_combo(ui, "vault_sync_dir", &mut dialog.draft.direction);
            ui.end_row();
            if dialog.draft.direction != Direction::Both {
                ui.label("Löschen");
                delete_combo(ui, "vault_sync_del", &mut dialog.draft.delete_policy);
                ui.end_row();
            }
        });
    if let Some(issue) = dialog.draft.issue() {
        ui.small(issue);
    }
    if matches!(
        dialog.draft.target,
        TargetChoice::Replica | TargetChoice::Folder
    ) {
        ui.small("Die zweite Seite wird beim ersten Lauf gefüllt.");
    }
    if let Some(error) = &dialog.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
}

fn direction_combo(ui: &mut egui::Ui, salt: &str, direction: &mut Direction) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(direction_label(*direction))
        .show_ui(ui, |ui| {
            for choice in [Direction::Both, Direction::AtoB, Direction::BtoA] {
                ui.selectable_value(direction, choice, direction_label(choice));
            }
        });
}

fn delete_combo(ui: &mut egui::Ui, salt: &str, policy: &mut crate::bisync::DeletePolicy) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(delete_label(*policy))
        .show_ui(ui, |ui| {
            for choice in ONE_SIDED_DELETE {
                ui.selectable_value(policy, choice, delete_label(choice));
            }
        });
}

/// Whether a draft of the job editor is the draft of a vault job: only a
/// vault job has the sides `vault-store://<site>` (a general job refuses
/// them in its checks).
pub(in crate::app) fn is_vault_draft(editor: &JobEditor) -> bool {
    editor.source.starts_with(STORE_PREFIX) && editor.target.starts_with(STORE_PREFIX)
}

fn site_label(rows: &[VaultRow], side: &str) -> String {
    let site = side.strip_prefix(STORE_PREFIX).unwrap_or(side);
    rows.iter()
        .find(|row| row.site == site)
        .map_or_else(|| site.to_string(), |row| row.label.clone())
}

/// The first page of the editor for a vault job (in the grid of the page):
/// the sides, the name, the direction, the deletion policy of a one-sided
/// job, „Aktiv“, and the fixed values.
pub(in crate::app) fn tresor_page(ui: &mut egui::Ui, editor: &mut JobEditor) {
    let rows = vault_feed::rows();
    ui.label("Ort A");
    ui.add_enabled(false, egui::Label::new(site_label(&rows, &editor.source)))
        .on_disabled_hover_text(FIXED_HOVER);
    ui.end_row();
    ui.label("Ort B");
    ui.add_enabled(false, egui::Label::new(site_label(&rows, &editor.target)))
        .on_disabled_hover_text(FIXED_HOVER);
    ui.end_row();
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(&mut editor.name).desired_width(360.0));
    ui.end_row();
    ui.label("Richtung");
    direction_combo(ui, "vault_edit_dir", &mut editor.direction);
    ui.end_row();
    editor.delete_policy = normalize(editor.direction, editor.delete_policy);
    editor.move_files = false;
    let model = editor_model_for(editor.direction, editor.compare);
    if model.editable.contains(&FieldId::DeletePolicy) {
        ui.label("Löschen");
        delete_combo(ui, "vault_edit_del", &mut editor.delete_policy);
        ui.end_row();
    }
    if model.editable.contains(&FieldId::Enabled) {
        ui.label("Aktiv");
        ui.checkbox(&mut editor.enabled, "Zeitplan aktiv");
        ui.end_row();
    }
    for (label, value) in model.fixed {
        ui.label(label);
        ui.add_enabled(false, egui::Label::new(value))
            .on_disabled_hover_text(FIXED_HOVER);
        ui.end_row();
    }
}

/// The pages „Vergleich & Versionen“, „Filter“ and „Übertragung & Befehle“
/// of a vault job: nothing to choose.
pub(in crate::app) fn fixed_page(ui: &mut egui::Ui) {
    ui.label("Bei Tresor-Jobs fest.");
    ui.end_row();
    ui.label("Die festen Werte stehen auf der Seite „Allgemein“.");
    ui.end_row();
}

/// The sides of a vault job for the job window: „A: Bezeichnung → B: Bezeichnung“.
pub(in crate::app) fn job_sides_text(job: &SyncJob) -> String {
    let rows = vault_feed::rows();
    format!(
        "A: {}  →  B: {}",
        site_label(&rows, &job.vault_site_a),
        site_label(&rows, &job.vault_site_b)
    )
}

/// The vault jobs that cannot be read, as lines for the job window („id:
/// fehlerhaft: Grund“). The files are read again every few seconds, not in
/// every frame.
pub(in crate::app) fn broken_lines() -> Vec<String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Vec<String>)>>> = OnceLock::new();
    let mut cache = guard(CACHE.get_or_init(Mutex::default));
    if let Some((at, lines)) = cache.as_ref() {
        if at.elapsed() < BROKEN_EVERY {
            return lines.clone();
        }
    }
    let lines = crate::syncjobs::load_report()
        .map(|report| broken_vault_job_lines(&report))
        .unwrap_or_default();
    *cache = Some((Instant::now(), lines.clone()));
    lines
}
