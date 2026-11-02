//! The connection between the vault windows and the main window (U29): the
//! section in the sidebar, the effects the windows ask for, the effect of a
//! lock on tabs (U29-2) and the folder picker the windows borrow.
//!
//! The state of the windows lives in the modules of the windows themselves
//! (a `static` each, like `share_screen_tab_ui`), because `App` is not part
//! of this package; the main window calls `ui_sidebar_vaults` once per frame
//! and that call drives everything else.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use eframe::egui;

use super::vault_feed::{self, guard};
use super::vault_models::{apply_lock_event, RowState, TabEffect, TabModel, VaultRow};
use super::vault_models_manage::is_vault_file_name;
use super::vault_sidebar_ui::{section_ui, Effect, PickTarget, SectionInfo, SidebarAction};
use super::{
    vault_actions_ui, vault_check_ui, vault_compare_ui, vault_create_ui, vault_settings_ui,
    vault_sync_editor_ui, vault_unlock_ui,
};
use crate::app::App;

/// How long the answer „is the drive runtime ready“ is trusted (the check
/// looks at the system).
const DRIVE_CHECK_EVERY: Duration = Duration::from_secs(10);

fn drive_ready() -> bool {
    static CACHE: OnceLock<Mutex<Option<(Instant, bool)>>> = OnceLock::new();
    let mut cache = guard(CACHE.get_or_init(Mutex::default));
    if let Some((at, ready)) = *cache {
        if at.elapsed() < DRIVE_CHECK_EVERY {
            return ready;
        }
    }
    let ready = crate::mount::drive_mount_supported() && crate::mount::drive_runtime_ready();
    *cache = Some((Instant::now(), ready));
    ready
}

/// The folder picker a window borrowed: the folder arrives in `copy_dest`
/// (the destination field of the copy dialog, which is closed while a vault
/// window picks), so the picker needs no purpose of its own. The previous
/// value comes back afterwards.
struct Picking {
    target: PickTarget,
    previous: String,
}

fn picking() -> &'static Mutex<Option<Picking>> {
    static PICKING: OnceLock<Mutex<Option<Picking>>> = OnceLock::new();
    PICKING.get_or_init(Mutex::default)
}

impl App {
    /// Draws the section „Tresore“ and drives the vault windows (once per
    /// frame, called by `ui_sidebar`).
    pub(in crate::app) fn ui_sidebar_vaults(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        vault_feed::start(&ctx);
        let snapshot = vault_feed::snapshot();
        for site in &snapshot.locked {
            let label = snapshot
                .rows
                .iter()
                .find(|row| &row.site == site)
                .map_or_else(|| site.clone(), |row| row.label.clone());
            self.on_vault_locked(site, &label);
        }
        self.vault_windows(&ctx);
        let info = self.vault_section_info(&snapshot.rows, snapshot.error);
        let actions = section_ui(ui, &snapshot.rows, &info);
        let mut effects = Vec::new();
        for action in actions {
            effects.extend(self.vault_action(&ctx, action, &snapshot.rows));
        }
        self.apply_vault_effects(&ctx, effects);
    }

    fn vault_section_info(&self, rows: &[VaultRow], error: Option<String>) -> SectionInfo {
        let mut jobs = Vec::new();
        for job in self.sync_jobs.iter().filter(|job| job.is_vault()) {
            for site in [&job.vault_site_a, &job.vault_site_b] {
                jobs.push((site.clone(), job.id.clone(), job.name.clone()));
            }
        }
        SectionInfo {
            any_unlocked: rows.iter().any(|row| row.state == RowState::Unlocked),
            drive_ready: drive_ready(),
            jobs,
            hint: self.vault_folder_hint(rows),
            error,
        }
    }

    /// T2: the open folder holds `vault.cryptomator` and is not entered yet.
    fn vault_folder_hint(&self, rows: &[VaultRow]) -> Option<String> {
        if self.recursive || self.root_path.is_empty() {
            return None;
        }
        if !self.folder_holds_vault() {
            return None;
        }
        let location = self.location_key(&self.root_path);
        let key = crate::vault::location_key(&location);
        let entered = rows
            .iter()
            .any(|row| crate::vault::location_key(&row.record.location) == key);
        (!entered).then_some(location)
    }

    /// Whether the open folder has a file `vault.cryptomator`; the answer is
    /// kept while the folder and its entry count stay as they are.
    fn folder_holds_vault(&self) -> bool {
        static CACHE: OnceLock<Mutex<(String, usize, bool)>> = OnceLock::new();
        let mut cache = guard(CACHE.get_or_init(Mutex::default));
        if cache.0 == self.root_path && cache.1 == self.entries.len() {
            return cache.2;
        }
        let holds = self
            .entries
            .iter()
            .any(|entry| !entry.is_dir && is_vault_file_name(&entry.name));
        *cache = (self.root_path.clone(), self.entries.len(), holds);
        holds
    }

    fn vault_windows(&mut self, ctx: &egui::Context) {
        let mut effects = vault_unlock_ui::show(ctx);
        effects.extend(vault_actions_ui::show(ctx));
        effects.extend(vault_create_ui::show(ctx, &self.saved_connections));
        effects.extend(vault_settings_ui::show(ctx));
        effects.extend(vault_compare_ui::show(ctx));
        effects.extend(vault_sync_editor_ui::show(ctx));
        self.finish_vault_pick();
        self.drain_vault_mount();
        self.apply_vault_effects(ctx, effects);
    }

    fn vault_action(
        &mut self,
        ctx: &egui::Context,
        action: SidebarAction,
        rows: &[VaultRow],
    ) -> Vec<Effect> {
        let row_of = |site: &str| rows.iter().find(|row| row.site == site);
        let label_of =
            |site: &str| row_of(site).map_or_else(|| site.to_string(), |r| r.label.clone());
        match action {
            SidebarAction::Open(site) => {
                self.open_vault_location(&crate::vault::vault_location(&site, "/"));
            }
            SidebarAction::OpenLocked(site) => {
                return vec![Effect::AskUnlock {
                    site,
                    then_open: true,
                }];
            }
            SidebarAction::Unlock(site) => {
                return vec![Effect::AskUnlock {
                    site,
                    then_open: false,
                }];
            }
            SidebarAction::Lock(site) => {
                return vault_unlock_ui::lock(ctx, Some(&site), &label_of(&site), false);
            }
            SidebarAction::LockAll => {
                return vault_unlock_ui::lock(ctx, None, "Alle Tresore", false);
            }
            SidebarAction::MountDrive(site) => {
                return vault_actions_ui::mount_drive(ctx, &site, &label_of(&site));
            }
            SidebarAction::Sync(site) => return vec![Effect::NewSync(site)],
            SidebarAction::Compare { job, name } => {
                return vec![Effect::OpenCompare { job, name }];
            }
            SidebarAction::Properties(site) => return vec![Effect::OpenSettings(site)],
            SidebarAction::Remove(site) => vault_actions_ui::ask_remove(&site, &label_of(&site)),
            SidebarAction::NewVault => vault_create_ui::open_create(),
            SidebarAction::AddVault(prefill) => vault_create_ui::open_add(prefill),
            SidebarAction::Reason(text) => return vec![Effect::Notice(text)],
        }
        Vec::new()
    }

    /// Carries out what the windows asked for.
    pub(in crate::app) fn apply_vault_effects(
        &mut self,
        ctx: &egui::Context,
        effects: Vec<Effect>,
    ) {
        for effect in effects {
            match effect {
                Effect::Notice(text) => self.notice = Some((text, Instant::now())),
                Effect::Error(text) => self.error_msg = Some(text),
                Effect::Unlocked { site, then_open } => {
                    vault_feed::poke();
                    if then_open {
                        self.open_vault_location(&crate::vault::vault_location(&site, "/"));
                    } else {
                        self.notice = Some(("Tresor entsperrt.".to_string(), Instant::now()));
                    }
                }
                Effect::Locked(_) => vault_feed::poke(),
                Effect::Removed(site) => {
                    vault_settings_ui::close_site(&site);
                    vault_compare_ui::on_locked(&site);
                    vault_feed::poke();
                }
                Effect::ReloadJobs => self.reload_sync_jobs("Sync-Setups neu laden"),
                Effect::AskUnlock { site, then_open } => {
                    let rows = vault_feed::rows();
                    let row = rows.iter().find(|row| row.site == site);
                    let label = row.map_or_else(|| site.clone(), |row| row.label.clone());
                    let remembered = row.is_some_and(|row| row.record.remember_password);
                    vault_unlock_ui::ask(&site, &label, remembered, remembered, then_open);
                }
                Effect::OpenCompare { job, name } => vault_compare_ui::open(ctx, &job, &name),
                Effect::OpenSettings(site) => vault_settings_ui::open(&site),
                Effect::RunJob(id) => self.run_job(&id),
                Effect::EditJob(id) => {
                    if let Some(job) = self.sync_jobs.iter().find(|job| job.id == id) {
                        self.job_editor = Some(crate::syncjobs::editor::JobEditor::from_job(job));
                    }
                }
                Effect::NewSync(site) => vault_sync_editor_ui::open_create(&site),
                Effect::PickFolder(target) => self.start_vault_pick(target),
                Effect::OpenVaultFile { site, path } => self.open_vault_path(&site, &path),
            }
        }
    }

    /// Opens the picker for a window; the folder comes back through
    /// `finish_vault_pick`.
    fn start_vault_pick(&mut self, target: PickTarget) {
        let mut slot = guard(picking());
        if slot.is_some() || self.picker.is_some() {
            self.notice = Some((
                "Die Ordnerauswahl ist schon geöffnet.".to_string(),
                Instant::now(),
            ));
            return;
        }
        let previous = std::mem::take(&mut self.copy_dest);
        *slot = Some(Picking { target, previous });
        drop(slot);
        self.open_picker(crate::app::picker_types::PickerPurpose::CopyDest, "");
    }

    /// Hands a finished pick to its window and restores `copy_dest`.
    fn finish_vault_pick(&mut self) {
        let mut slot = guard(picking());
        if self.picker.is_some() {
            return;
        }
        let Some(done) = slot.take() else {
            return;
        };
        drop(slot);
        let chosen = std::mem::replace(&mut self.copy_dest, done.previous);
        if chosen.trim().is_empty() {
            return;
        }
        match done.target {
            PickTarget::CreateLocation => vault_create_ui::picked(true, chosen),
            PickTarget::AddLocation => vault_create_ui::picked(false, chosen),
            PickTarget::SyncFolder => vault_sync_editor_ui::picked(chosen),
        }
    }

    /// U29-2: a lock ends the access to the site. Every tab of the site is
    /// cleared (and its history, which holds names of the vault); windows
    /// that hold names of the vault drop them.
    pub(in crate::app) fn on_vault_locked(&mut self, site: &str, label: &str) {
        let mut tabs: Vec<TabModel> = (0..self.tabs.len())
            .map(|index| TabModel {
                prefix: self.vault_tab_prefix(index),
            })
            .collect();
        for effect in apply_lock_event(&mut tabs, site, label) {
            match effect {
                TabEffect::ClearWithNotice { tab, text } => {
                    self.clear_vault_tab(tab);
                    self.notice = Some((text, Instant::now()));
                }
            }
        }
        vault_compare_ui::on_locked(site);
        vault_check_ui::on_locked(site);
    }

    fn vault_tab_prefix(&self, index: usize) -> Option<String> {
        let remote = if index == self.active_tab {
            self.remote.as_ref()
        } else {
            self.tabs.get(index).and_then(|tab| tab.remote.as_ref())
        };
        remote.and_then(|remote| remote.endpoint_prefix.clone())
    }

    fn clear_vault_tab(&mut self, index: usize) {
        if index == self.active_tab {
            self.show_landing_page();
            self.history.clear();
            self.forward.clear();
        } else if let Some(tab) = self.tabs.get_mut(index) {
            *tab = crate::app::app_models::TabState::default();
        }
    }
}
