//! Shared drive projection and identity-aware UI/index reconciliation.
//! OS inventory and event APIs remain behind platform_helpers.
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use crate::app::prelude::*;
use crate::app::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriveEntry {
    pub root: String,
    /// None means the OS cannot currently prove a media identity (for example
    /// an empty optical drive). A letter alone never becomes a volume identity.
    pub identity: Option<String>,
    /// Current attachment, distinct from a reusable media identity. Never
    /// persisted or used to authorize a reconnect to an unknown provider.
    pub incarnation: Option<String>,
    pub free: u64,
    pub total: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DriveBinding {
    root: String,
    identity: Option<String>,
    disconnected: bool,
}

fn binding_for(entries: &[DriveEntry], path: &str) -> Option<DriveBinding> {
    entries.iter().filter(|entry| contains(&entry.root, path)).max_by_key(|entry| entry.root.len())
        .map(|entry| DriveBinding { root: entry.root.clone(), identity: entry.identity.clone(), disconnected: false })
}

fn binding_available(binding: &DriveBinding, entries: &[DriveEntry]) -> bool {
    !binding.disconnected || entries.iter().any(|entry|
        entry.root == binding.root && entry.identity.is_some() && entry.identity == binding.identity)
}

#[derive(Default)]
pub(crate) struct DriveChanges { pub removed: Vec<String>, pub added: Vec<String> }

pub(crate) fn diff(old: &[DriveEntry], new: &[DriveEntry]) -> DriveChanges {
    let same = |a: &DriveEntry, b: &DriveEntry| a.root == b.root
        && a.identity == b.identity && a.incarnation == b.incarnation;
    DriveChanges {
        removed: old.iter().filter(|a| !new.iter().any(|b| same(a, b))).map(|a| a.root.clone()).collect(),
        added: new.iter().filter(|b| !old.iter().any(|a| same(a, b))).map(|b| b.root.clone()).collect(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DriveConsumer { Analytics, Reclaim, Picker }

struct SourceBinding { source: String, binding: Option<DriveBinding> }

struct Delta {
    roots: Vec<DriveEntry>,
    generations: BTreeMap<String, u64>,
    rx: Receiver<IndexMsg>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Delta { fn drop(&mut self) { self.cancel.store(true, Ordering::Release); } }

pub(crate) struct DriveState {
    enabled: bool,
    initialized: bool,
    index_enabled: bool,
    monitor: Option<super::drive_events::DriveMonitor>,
    pub entries: Vec<DriveEntry>,
    focused: bool,
    retry_at: Instant,
    /// Only affected roots get a new generation; unrelated index paths survive.
    pub generations: BTreeMap<String, u64>,
    pending: BTreeMap<String, DriveEntry>,
    consumers: BTreeMap<DriveConsumer, SourceBinding>,
    delta: Option<Delta>,
    pub index_obsolete: bool,
}

impl DriveState {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, initialized: false, index_enabled: false, monitor: None, entries: Vec::new(), focused: false,
            retry_at: Instant::now(), generations: BTreeMap::new(),
            pending: BTreeMap::new(), consumers: BTreeMap::new(), delta: None, index_obsolete: false }
    }
}

/// Match a complete path segment; /media/a must not contain /media/ab.
pub(crate) fn contains(root: &str, path: &str) -> bool {
    let windows = root.as_bytes().get(1) == Some(&b':') || root.starts_with("\\\\");
    let (root, path) = if windows { (root.replace('\\', "/").to_lowercase(), path.replace('\\', "/").to_lowercase()) }
        else { (root.to_string(), path.to_string()) };
    let root = root.trim_end_matches('/');
    path == root || path.strip_prefix(root).is_some_and(|tail| tail.starts_with('/'))
}

impl App {
    pub(in crate::app) fn update_drives(&mut self, ctx: &egui::Context) {
        if !self.drive_state.enabled { return; }
        if self.drive_state.monitor.is_none() && Instant::now() >= self.drive_state.retry_at {
            match super::drive_events::DriveMonitor::start(ctx.clone()) {
                Ok(monitor) => self.drive_state.monitor = Some(monitor),
                Err(error) => {
                    self.drive_state.retry_at = Instant::now() + std::time::Duration::from_secs(30);
                    self.push_app_error("Laufwerksbestand", error);
                }
            }
        }
        let focused = ctx.input(|input| input.focused);
        if focused && !self.drive_state.focused {
            if let Some(monitor) = &self.drive_state.monitor { monitor.refresh(); }
        }
        self.drive_state.focused = focused;
        let message = self.drive_state.monitor.as_ref().and_then(|monitor| monitor.snapshots.try_recv().ok());
        if let Some(message) = message {
            match message {
                Ok(entries) => self.apply_drive_snapshot(entries),
                // Keep the last successful snapshot on inventory failure.
                Err(error) => {
                    if self.drive_state.retry_at <= Instant::now() {
                        self.push_app_error("Laufwerksbestand", error);
                        self.drive_state.retry_at = Instant::now() + std::time::Duration::from_secs(30);
                    }
                }
            }
        }
        if self.remote.is_none() && self.disconnected_drive(&self.root_path) {
            self.error_msg = Some("Datenträger nicht verbunden".into());
        }
        self.drain_drive_index();
        self.start_pending_drive_index();
        // Also discovers an event thread/worker startup failure without UI activity.
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
    }

    pub(in crate::app) fn apply_drive_snapshot(&mut self, entries: Vec<DriveEntry>) {
        let initial_inventory = !self.drive_state.initialized;
        let changes = diff(&self.drive_state.entries, &entries);
        if !self.drive_state.initialized {
            self.drive_state.index_enabled = !self.folder_index.is_empty();
        }
        for root in &changes.removed {
            let generation = self.drive_state.generations.entry(root.clone()).or_default();
            *generation = generation.wrapping_add(1);
            self.drive_state.pending.remove(root);
        }
        for root in &changes.added {
            if let Some(entry) = entries.iter().find(|entry| &entry.root == root) {
                if self.drive_state.initialized && self.drive_state.index_enabled {
                    self.drive_state.pending.insert(root.clone(), entry.clone());
                }
            }
        }
        if !changes.removed.is_empty() {
            self.disconnect_drive_views(&changes.removed);
            let stale: Vec<_> = self.folder_index.iter().filter(|path|
                changes.removed.iter().any(|root| contains(root, path))).cloned().collect();
            for path in stale { self.folder_index.remove(&path); }
            self.index_dirty = true;
            // Existing full builds own one aggregate result. Cancel that result,
            // preserve the current unaffected index, and requeue its mounted roots.
            if self.index_building {
                self.drive_state.index_obsolete = true;
                if let Some(cancel) = &self.index_cancel { cancel.store(true, Ordering::Release); }
                for entry in &entries { self.drive_state.pending.insert(entry.root.clone(), entry.clone()); }
            }
        }
        // Sidebar, picker, analytics, reclaim and landing use drive_info;
        // omni targets, share exports and scan roots use drives. Publish together.
        self.drives = entries.iter().map(|entry| entry.root.clone()).collect();
        self.drive_info = entries.iter().map(|entry| (entry.root.clone(), entry.free, entry.total)).collect();
        self.drive_state.entries = entries;
        if !self.drive_state.initialized {
            for state in self.drive_state.consumers.values_mut() {
                if state.binding.is_none() { state.binding = binding_for(&self.drive_state.entries, &state.source); }
            }
            self.bind_scan_drive();
            for tab in &mut self.tabs {
                if tab.remote.is_none() && tab.drive_binding.is_none() {
                    tab.drive_binding = binding_for(&self.drive_state.entries, &tab.root_path);
                }
            }
        }
        self.drive_state.initialized = true;
        if initial_inventory && self.drive_state.index_enabled {
            self.verify_loaded_optical_index();
        }
        if !changes.removed.is_empty() || !changes.added.is_empty() {
            self.update_drive_watches(&changes.removed, &changes.added);
            if !self.folder_search_query.is_empty() { self.run_folder_search(); }
            // The virtual peer tree still refreshes through its existing daemon
            // DeviceWatch. Refresh the mount presentation after the OS view changes.
            self.mount_ui.next_poll = Instant::now();
        }
    }

    fn disconnect_drive_views(&mut self, removed: &[String]) {
        let affected = |path: &str| removed.iter().any(|root| contains(root, path));
        let detail = "Datenträger nicht verbunden".to_string();
        for state in self.drive_state.consumers.values_mut() {
            if let Some(binding) = &mut state.binding {
                if removed.iter().any(|root| root == &binding.root) { binding.disconnected = true; }
            }
        }
        if self.remote.is_none() && affected(&self.root_path) {
            self.read_access.invalidate_partial();
            if let Some(binding) = &mut self.drive_binding { binding.disconnected = true; }
            if let Some(handle) = self.scan_handle.take() { handle.cancel.store(true, Ordering::Release); }
            self.scan_rx = None;
            self.scan_running = false;
            self.scan_was_canceled = true;
            // Stale rows must not address a replacement device at the same root.
            self.entries.clear(); self.view.clear(); self.tree.rows.clear();
            self.selection.clear(); self.last_anchor = None; self.cursor = None;
            self.error_msg = Some(detail.clone());
            self.failed_paths.push((self.root_path.clone(), detail.clone()));
        }
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if index != self.active_tab && tab.remote.is_none() && affected(&tab.root_path) {
                tab.read_access.invalidate_partial();
                if let Some(binding) = &mut tab.drive_binding { binding.disconnected = true; }
                if let Some(handle) = tab.scan_handle.take() { handle.cancel.store(true, Ordering::Release); }
                tab.scan_rx = None;
                tab.scan_running = false;
                tab.scan_was_canceled = true;
                tab.entries.clear(); tab.view.clear(); tab.tree.rows.clear();
                tab.selection.clear(); tab.last_anchor = None; tab.cursor = None;
                tab.failed_paths.push((tab.root_path.clone(), detail.clone()));
            }
        }
        if self.picker.as_ref().is_some_and(|p| !p.is_remote && affected(&p.cwd)) {
            if let Some(picker) = &mut self.picker {
                picker.list_rx = None;
                picker.listing = false;
                picker.entries.clear(); picker.selected = None;
                picker.error = Some(detail.clone());
            }
            self.error_msg = Some(detail.clone());
        }
        if self.analytics_source.as_ref().is_some_and(|source| matches!(source, StorageScanSource::Local { root } if affected(root))) {
            self.analytics_access.invalidate_partial();
            if let Some(scan) = self.analytics_scan.take() { scan.progress.cancel.store(true, Ordering::Release); }
            self.analytics_state = StorageRunState::Failed;
            self.analytics_tree = None;
            self.analytics_invalidate();
            self.analytics_issues.push(crate::analytics::ScanIssue {
                path: self.analytics_source.as_ref().map(|source| source.root().to_string()).unwrap_or_default(),
                detail: detail.clone(),
            });
        }
        if self.reclaim_source.as_ref().is_some_and(|source| matches!(source, StorageScanSource::Local { root } if affected(root))) {
            self.cancel_reclaim_scan();
            self.reclaim_state = StorageRunState::Failed;
            self.reclaim_report = None;
            self.reclaim_selected.clear();
            self.reclaim_issues.push(detail);
        }
    }

    /// Refresh must not adopt a different device mounted at the same path.
    /// Explicit navigation remains the user's way to select the new device.
    pub(in crate::app) fn disconnected_drive(&self, path: &str) -> bool {
        self.drive_binding.as_ref().is_some_and(|binding| {
            contains(&binding.root, path) && binding.disconnected &&
                !self.drive_state.entries.iter().any(|entry| entry.root == binding.root
                    && entry.identity.is_some() && entry.identity == binding.identity)
        })
    }

    pub(in crate::app) fn selecting_drive_root(&self, path: &str) -> bool {
        self.drive_state.entries.iter().any(|entry| contains(&entry.root, path) && contains(path, &entry.root))
    }

    pub(in crate::app) fn bind_scan_drive(&mut self) {
        self.drive_binding = if self.remote.is_none() {
            binding_for(&self.drive_state.entries, &self.root_path)
        } else { None };
    }

    /// Called only for a deliberate local source selection, never for refresh.
    pub(in crate::app) fn accept_drive_consumer(&mut self, consumer: DriveConsumer, root: &str) {
        let binding = binding_for(&self.drive_state.entries, root);
        self.drive_state.consumers.insert(consumer, SourceBinding { source: root.into(), binding });
    }

    pub(in crate::app) fn drive_consumer_available(&mut self, consumer: DriveConsumer, root: &str, remote: bool) -> bool {
        if remote { self.drive_state.consumers.remove(&consumer); return true; }
        let current = self.drive_state.consumers.get(&consumer);
        // A changed child path is still inside the pinned device. Reject it
        // before binding the new path if that device was disconnected/replaced.
        if current.and_then(|state| state.binding.as_ref()).is_some_and(|binding|
            contains(&binding.root, root) && !binding_available(binding, &self.drive_state.entries)) {
            return false;
        }
        if current.is_none_or(|state| state.source != root || state.binding.is_none()) {
            self.accept_drive_consumer(consumer, root);
        }
        self.drive_state.consumers.get(&consumer).and_then(|state| state.binding.as_ref())
            .is_none_or(|binding| binding_available(binding, &self.drive_state.entries))
    }

    pub(in crate::app) fn drive_index_ready(&self) -> bool {
        !self.drive_state.enabled || self.drive_state.initialized
    }

    pub(in crate::app) fn capture_index_drives(&mut self) {
        self.drive_state.index_obsolete = false;
        self.drive_state.index_enabled = true;
    }

    fn start_pending_drive_index(&mut self) {
        if self.index_building || self.drive_state.delta.is_some() || self.drive_state.pending.is_empty() { return; }
        // One changed root at a time: an unreadable medium must not discard
        // the successful index delta of an independent mounted root.
        let Some((_, entry)) = self.drive_state.pending.pop_first() else { return; };
        let roots = vec![entry];
        let paths = roots.iter().map(|entry| PathBuf::from(&entry.root)).collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let generations = self.drive_state.generations.clone();
        match super::drive_inventory::start_index_delta(paths, cancel.clone()) {
            Ok(rx) => self.drive_state.delta = Some(Delta { roots, generations, rx, cancel }),
            Err(error) => self.push_app_error("Laufwerksindex", error),
        }
    }

    fn drain_drive_index(&mut self) {
        let Some(delta) = self.drive_state.delta.take() else { return; };
        let message = match delta.rx.try_recv() {
            Ok(message) => message,
            Err(crossbeam_channel::TryRecvError::Empty) => { self.drive_state.delta = Some(delta); return; }
            Err(_) => { self.push_app_error("Laufwerksindex", "Indexworker wurde beendet".into()); return; }
        };
        match message {
            IndexMsg::Complete(index) => {
                let valid: Vec<_> = delta.roots.iter().filter(|root|
                    self.drive_state.entries.iter().any(|entry| entry.root == root.root
                        && entry.identity == root.identity && entry.incarnation == root.incarnation)
                    && delta.generations.get(&root.root) == self.drive_state.generations.get(&root.root)).collect();
                for path in index.iter().filter(|path| valid.iter().any(|root| contains(&root.root, path))) {
                    match self.folder_index.try_insert(path.clone()) {
                        Ok(changed) => self.index_dirty |= changed,
                        Err(error) => { self.push_app_error("Laufwerksindex", error.to_string()); break; }
                    }
                }
                if !self.folder_search_query.is_empty() { self.run_folder_search(); }
            }
            IndexMsg::Failed(error) => self.push_app_error("Laufwerksindex", error),
            _ => {}
        }
    }
}
