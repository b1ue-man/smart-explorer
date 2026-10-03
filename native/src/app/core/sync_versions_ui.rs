//! Desktop per-job version browser and explicit reversible restore confirmation.
use super::prelude::*;
use super::*;
use crate::bisync::{versions::VersionEntry, PairSide};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct VersionIdentity {
    pub source: String,
    pub target: String,
    pub root_a: String,
    pub root_b: String,
    pub pair: String,
    pub lock: String,
}
pub(in crate::app) struct VersionSnapshot {
    pub identity: VersionIdentity,
    pub entries: Vec<VersionEntry>,
    pub message: Option<String>,
    pub load_error: Option<String>,
}
pub(in crate::app) struct VersionTask {
    pub worker: Option<std::thread::JoinHandle<()>>,
    pub rx: Receiver<Result<VersionSnapshot, String>>,
    pub cancel: Arc<AtomicBool>,
    pub restoring: bool,
}
impl Drop for VersionTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
pub(in crate::app) struct SyncVersionsUi {
    id: String,
    identity: Option<VersionIdentity>,
    entries: Vec<VersionEntry>,
    selected: Option<(VersionEntry, PairSide)>,
    task: Option<VersionTask>,
    error: Option<String>,
}
impl SyncVersionsUi {
    pub(in crate::app) fn protects_job(&self, id: &str) -> bool {
        self.id == id && self.task.as_ref().is_some_and(|task| task.restoring)
    }
}
impl App {
    fn track_version_task(&mut self, mut task: VersionTask) -> VersionTask {
        if let Some(worker) = task.worker.take() {
            self.track_desktop_sync_worker(worker, task.cancel.clone());
        }
        task
    }

    pub(in crate::app) fn open_sync_versions(&mut self, id: &str) {
        if self
            .sync_versions
            .as_ref()
            .is_some_and(|ui| ui.task.as_ref().is_some_and(|task| task.restoring))
        {
            return;
        }
        let mut ui = SyncVersionsUi {
            id: id.into(),
            identity: None,
            entries: Vec::new(),
            selected: None,
            task: None,
            error: None,
        };
        match super::sync_versions_task::start(id.into(), None) {
            Ok(task) => ui.task = Some(self.track_version_task(task)),
            Err(error) => ui.error = Some(error),
        }
        self.sync_versions = Some(ui);
        self.show_sync_jobs = true;
    }

    pub(in crate::app) fn ui_sync_versions(&mut self, ctx: &egui::Context) {
        let Some(mut view) = self.sync_versions.take() else {
            return;
        };
        let polled = view.task.as_ref().map(|task| task.rx.try_recv());
        match polled {
            Some(Ok(result)) => {
                let restoring = view.task.take().is_some_and(|task| task.restoring);
                if restoring {
                    self.bisync_running = false;
                    self.bisync_cancel = None;
                }
                match result {
                    Ok(snapshot) => {
                        view.identity = Some(snapshot.identity);
                        view.entries = snapshot.entries;
                        view.error = snapshot.load_error;
                        view.selected = None;
                        if let Some(message) = snapshot.message {
                            self.notice = Some((message, Instant::now()));
                            if !self.root_path.is_empty() {
                                self.rescan();
                            }
                        }
                    }
                    Err(error) => view.error = Some(error),
                }
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                let restoring = view.task.take().is_some_and(|task| task.restoring);
                if restoring {
                    self.bisync_running = false;
                    self.bisync_cancel = None;
                }
                view.error = Some(
                    "Versionsaktion endete ohne Ergebnis; Liste neu laden und Original prüfen."
                        .into(),
                );
            }
            _ => {}
        }
        let mut open = true;
        let mut reload = false;
        let mut restore = false;
        let name = self
            .sync_jobs
            .iter()
            .find(|job| job.id == view.id)
            .map(|job| job.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Sync-Setup");
        egui::Window::new(format!("Versionen · {name}")).open(&mut open).collapsible(false)
            .resizable(true).default_size([780.0,500.0]).max_size(theme::window_content_limit(ctx))
            .show(ctx, |ui| {
                ui.label("Wiederherstellen betrifft die gewählte Seite. Die aktuelle Datei wird vorher als Version erhalten; der nächste Sync gleicht die andere Seite ab.");
                if let Some(identity) = &view.identity { ui.label(format!("{} ⇄ {}", identity.source, identity.target)); }
                if let Some(error) = &view.error { ui.colored_label(theme::danger(ui), error); }
                if let Some(task) = &view.task {
                    ui.horizontal(|ui| { ui.spinner(); ui.label(if task.restoring { "Version sicher wiederherstellen…" } else { "Versionen laden…" }); });
                    if ui.button("Abbrechen").clicked() { task.cancel.store(true, Ordering::Release); }
                    ctx.request_repaint_after(std::time::Duration::from_millis(100)); return;
                }
                if ui.button("Neu laden").clicked() { reload = true; }
                if view.entries.is_empty() { ui.label("Keine erhaltenen Versionen für dieses Setup gefunden."); }
                egui::ScrollArea::vertical().max_height(310.0).show_rows(ui,52.0,view.entries.len(),|ui, visible| {
                    for index in visible {
                        let entry = &view.entries[index];
                        ui.push_id(index, |ui| {
                            ui.add(egui::Label::new(RichText::new(&entry.rel).strong()).truncate());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{} · {} Bytes · {} · {}", fmt_ms(entry.preserved_ms), entry.size, reason(entry.reason), store(entry.store)));
                                for side in [PairSide::A,PairSide::B] {
                                    if entry.side.is_none() || entry.side == Some(side) {
                                        if ui.small_button(if side == PairSide::A { "Quelle wiederherstellen…" } else { "Ziel wiederherstellen…" }).clicked() {
                                            view.selected = Some((entry.clone(),side));
                                        }
                                    }
                                }
                            });
                        });
                    }
                });
                if let Some((entry,side)) = &view.selected {
                    ui.separator(); ui.label(format!("„{}“ vom {} auf {} wiederherstellen?", entry.rel, fmt_ms(entry.preserved_ms), side.label()));
                    if entry.side.is_none() { ui.colored_label(theme::warning(ui), "Alte Version ohne gespeicherte Seitenzuordnung; Ziel bewusst auswählen."); }
                    if ui.add_enabled(!self.bisync_running && !self.sync_running && self.merge.is_none() && self.conflict_resolution.is_none(),
                        egui::Button::new("Version jetzt wiederherstellen")).clicked() { restore = true; }
                }
            });
        if restore {
            if let (Some(identity), Some((entry, side))) =
                (view.identity.clone(), view.selected.clone())
            {
                match super::sync_versions_task::start(
                    view.id.clone(),
                    Some((identity, entry, side)),
                ) {
                    Ok(task) => {
                        self.bisync_running = true;
                        self.bisync_cancel = Some(task.cancel.clone());
                        view.task = Some(self.track_version_task(task));
                        view.error = None;
                    }
                    Err(error) => view.error = Some(error),
                }
            }
        } else if reload {
            view.selected = None;
            match super::sync_versions_task::start(view.id.clone(), None) {
                Ok(task) => {
                    view.task = Some(self.track_version_task(task));
                    view.error = None;
                }
                Err(error) => view.error = Some(error),
            }
        }
        if !open {
            if let Some(task) = &view.task {
                task.cancel.store(true, Ordering::Release);
            }
        }
        if open || view.task.as_ref().is_some_and(|task| task.restoring) {
            self.sync_versions = Some(view);
        }
    }
}
fn reason(reason: Option<crate::bisync::versions::VersionReason>) -> &'static str {
    use crate::bisync::versions::VersionReason::*;
    match reason {
        Some(Replaced) => "ersetzt",
        Some(Deleted) => "gelöscht",
        Some(Resolved) => "Konflikt",
        Some(Restored) => "vor Wiederherstellung",
        None => "Altbestand",
    }
}
fn store(store: crate::bisync::versions::VersionStore) -> &'static str {
    match store {
        crate::bisync::versions::VersionStore::SyncRoot => "Sync-Ordner",
        crate::bisync::versions::VersionStore::AppData => "App-Daten",
    }
}
