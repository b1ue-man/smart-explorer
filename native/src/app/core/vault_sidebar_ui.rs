//! The sidebar section „Tresore“ (U29-4): one row per vault site with its
//! state mark, the context menu (right click and „⋯“), the two small buttons
//! „Tresor hinzufügen…“ and „Neuer Tresor…“ and the hint bar for a folder
//! that holds a vault (T2). The section only draws and returns the actions
//! the user chose; `vault_glue.rs` carries them out.
use eframe::egui;

use super::vault_models::{row_hover, row_text, RowState, VaultRow};

/// What the windows and threads ask the main window to do.
#[derive(Clone, Debug)]
pub(crate) enum Effect {
    Notice(String),
    Error(String),
    /// A site is unlocked; `then_open` opens it as a tab.
    Unlocked {
        site: String,
        then_open: bool,
    },
    Locked(Vec<String>),
    /// A site left the list.
    Removed(String),
    ReloadJobs,
    AskUnlock {
        site: String,
        then_open: bool,
    },
    OpenCompare {
        job: String,
        name: String,
    },
    /// Open the properties of a site.
    OpenSettings(String),
    /// Start the vault job now (the runner of vault jobs).
    RunJob(String),
    /// Open the editor „Tresor-Sync“ of a vault job.
    EditJob(String),
    /// Open the dialog „Synchronisieren…“ for a site.
    NewSync(String),
    /// Open the folder picker for one of the windows.
    PickFolder(PickTarget),
    /// Open a file of a vault site by the open rule (C7.2).
    OpenVaultFile {
        site: String,
        path: String,
    },
}

/// Which window a picked folder goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickTarget {
    CreateLocation,
    AddLocation,
    SyncFolder,
}

/// What the user chose in the section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarAction {
    Open(String),
    /// A click on a locked row: unlock, then open it as a tab.
    OpenLocked(String),
    /// „Entsperren…“ of the menu: unlock only.
    Unlock(String),
    Lock(String),
    LockAll,
    MountDrive(String),
    Sync(String),
    Compare {
        job: String,
        name: String,
    },
    Properties(String),
    Remove(String),
    NewVault,
    AddVault(Option<String>),
    /// A click on an unavailable site: say why.
    Reason(String),
}

/// What the section needs besides the rows.
#[derive(Clone, Debug, Default)]
pub(crate) struct SectionInfo {
    pub(crate) any_unlocked: bool,
    /// The drive runtime is ready (Dokany, FUSE).
    pub(crate) drive_ready: bool,
    /// The vault jobs: (site, job ID, job name), one entry per side.
    pub(crate) jobs: Vec<(String, String, String)>,
    /// The location of the open folder when it holds a vault.
    pub(crate) hint: Option<String>,
    /// Why the list or the worker could not be read.
    pub(crate) error: Option<String>,
}

fn menu(ui: &mut egui::Ui, row: &VaultRow, info: &SectionInfo, out: &mut Vec<SidebarAction>) {
    let unlocked = row.state == RowState::Unlocked;
    let mut chosen: Option<SidebarAction> = None;
    if unlocked && ui.button("Öffnen").clicked() {
        chosen = Some(SidebarAction::Open(row.site.clone()));
    }
    if row.state == RowState::Locked && ui.button("Entsperren…").clicked() {
        chosen = Some(SidebarAction::Unlock(row.site.clone()));
    }
    if unlocked && ui.button("Sperren").clicked() {
        chosen = Some(SidebarAction::Lock(row.site.clone()));
    }
    if info.any_unlocked && ui.button("Alle sperren").clicked() {
        chosen = Some(SidebarAction::LockAll);
    }
    ui.separator();
    if unlocked && info.drive_ready && ui.button("Als Laufwerk einhängen").clicked() {
        chosen = Some(SidebarAction::MountDrive(row.site.clone()));
    }
    if row.state != RowState::Unavailable && ui.button("Synchronisieren…").clicked() {
        chosen = Some(SidebarAction::Sync(row.site.clone()));
    }
    let jobs: Vec<&(String, String, String)> =
        info.jobs.iter().filter(|job| job.0 == row.site).collect();
    match jobs.as_slice() {
        [] => {}
        [(_, id, name)] => {
            if ui.button("Vergleichen…").clicked() {
                chosen = Some(SidebarAction::Compare {
                    job: id.clone(),
                    name: name.clone(),
                });
            }
        }
        many => {
            ui.menu_button("Vergleichen…", |ui| {
                for (_, id, name) in many {
                    if ui.button(name).clicked() {
                        chosen = Some(SidebarAction::Compare {
                            job: id.clone(),
                            name: name.clone(),
                        });
                        ui.close_menu();
                    }
                }
            });
        }
    }
    ui.separator();
    if ui.button("Eigenschaften…").clicked() {
        chosen = Some(SidebarAction::Properties(row.site.clone()));
    }
    if ui.button("Austragen…").clicked() {
        chosen = Some(SidebarAction::Remove(row.site.clone()));
    }
    if let Some(action) = chosen {
        out.push(action);
        ui.close_menu();
    }
}

/// Draws the section and returns the actions chosen in this frame.
pub(crate) fn section_ui(
    ui: &mut egui::Ui,
    rows: &[VaultRow],
    info: &SectionInfo,
) -> Vec<SidebarAction> {
    let mut out = Vec::new();
    crate::app::theme::section(ui, "TRESORE");
    if rows.is_empty() {
        ui.small("Noch kein Tresor eingetragen.");
    }
    for row in rows {
        ui.horizontal(|ui| {
            let text = format!("{}  ({})", row.label, row.state.title());
            let width = (ui.available_width() - 24.0).max(1.0);
            let response = crate::app::sidebar_locations::sidebar_row(ui, &text, width, false)
                .on_hover_text(row_hover(&row.label, row.state, row.reason.as_deref()));
            if response.clicked() {
                out.push(match row.state {
                    RowState::Locked => SidebarAction::OpenLocked(row.site.clone()),
                    RowState::Unlocked | RowState::Locking => SidebarAction::Open(row.site.clone()),
                    RowState::Unavailable => SidebarAction::Reason(format!(
                        "Tresor „{}“: {}",
                        row.label,
                        row_text(row.state, row.reason.as_deref())
                    )),
                });
            }
            response.context_menu(|ui| menu(ui, row, info, &mut out));
            ui.menu_button("⋯", |ui| menu(ui, row, info, &mut out));
        });
    }
    if let Some(error) = &info.error {
        ui.colored_label(crate::app::theme::warning(ui), error.as_str());
    }
    if let Some(location) = &info.hint {
        ui.small("Dieser Ordner enthält einen Tresor.");
        if ui.small_button("Als Tresor eintragen…").clicked() {
            out.push(SidebarAction::AddVault(Some(location.clone())));
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Tresor hinzufügen…").clicked() {
            out.push(SidebarAction::AddVault(None));
        }
        if ui.small_button("Neuer Tresor…").clicked() {
            out.push(SidebarAction::NewVault);
        }
    });
    out
}
