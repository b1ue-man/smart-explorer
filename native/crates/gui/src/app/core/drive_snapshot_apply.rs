//! Apply one drive inventory and invalidate only its affected consumers.
use crate::app::prelude::*;
use crate::app::*;
use std::sync::atomic::Ordering;

use super::{binding_for, contains, diff, DriveEntry};

impl App {
    pub(in crate::app) fn apply_drive_snapshot(&mut self, entries: Vec<DriveEntry>) {
        let initial_inventory = !self.drive_state.initialized;
        let changes = diff(&self.drive_state.entries, &entries);
        if !self.drive_state.initialized {
            self.drive_state.index_enabled = !self.folder_index.is_empty();
        }
        for root in &changes.removed {
            let generation = self
                .drive_state
                .generations
                .entry(root.clone())
                .or_default();
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
            let stale: Vec<_> = self
                .folder_index
                .iter()
                .filter(|path| changes.removed.iter().any(|root| contains(root, path)))
                .cloned()
                .collect();
            for path in stale {
                self.folder_index.remove(&path);
            }
            self.index_dirty = true;
            // Existing full builds own one aggregate result. Cancel that result,
            // preserve the current unaffected index, and requeue its mounted roots.
            if self.index_building {
                self.drive_state.index_obsolete = true;
                if let Some(cancel) = &self.index_cancel {
                    cancel.store(true, Ordering::Release);
                }
                for entry in &entries {
                    self.drive_state
                        .pending
                        .insert(entry.root.clone(), entry.clone());
                }
            }
        }
        // Sidebar, picker, analytics, reclaim and landing use drive_info;
        // omni targets, share exports and scan roots use drives. Publish together.
        self.drives = entries.iter().map(|entry| entry.root.clone()).collect();
        self.drive_info = entries
            .iter()
            .map(|entry| (entry.root.clone(), entry.free, entry.total))
            .collect();
        self.drive_state.entries = entries;
        if !self.drive_state.initialized {
            for state in self.drive_state.consumers.values_mut() {
                if state.binding.is_none() {
                    state.binding = binding_for(&self.drive_state.entries, &state.source);
                }
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
            if !self.folder_search_query.is_empty() {
                self.run_folder_search();
            }
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
                if removed.iter().any(|root| root == &binding.root) {
                    binding.disconnected = true;
                }
            }
        }
        if self.remote.is_none() && affected(&self.root_path) {
            self.read_access.invalidate_partial();
            if let Some(binding) = &mut self.drive_binding {
                binding.disconnected = true;
            }
            if let Some(handle) = self.scan_handle.take() {
                handle.cancel.store(true, Ordering::Release);
            }
            self.scan_rx = None;
            self.scan_running = false;
            self.scan_was_canceled = true;
            // Stale rows must not address a replacement device at the same root.
            self.entries.clear();
            self.view.clear();
            self.tree.rows.clear();
            self.selection.clear();
            self.last_anchor = None;
            self.cursor = None;
            self.error_msg = Some(detail.clone());
            self.failed_paths
                .push((self.root_path.clone(), detail.clone()));
        }
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if index != self.active_tab && tab.remote.is_none() && affected(&tab.root_path) {
                tab.read_access.invalidate_partial();
                if let Some(binding) = &mut tab.drive_binding {
                    binding.disconnected = true;
                }
                if let Some(handle) = tab.scan_handle.take() {
                    handle.cancel.store(true, Ordering::Release);
                }
                tab.scan_rx = None;
                tab.scan_running = false;
                tab.scan_was_canceled = true;
                tab.entries.clear();
                tab.view.clear();
                tab.tree.rows.clear();
                tab.selection.clear();
                tab.last_anchor = None;
                tab.cursor = None;
                tab.failed_paths
                    .push((tab.root_path.clone(), detail.clone()));
            }
        }
        if self
            .picker
            .as_ref()
            .is_some_and(|p| !p.is_remote && affected(&p.cwd))
        {
            if let Some(picker) = &mut self.picker {
                picker.list_rx = None;
                picker.listing = false;
                picker.entries.clear();
                picker.selected = None;
                picker.error = Some(detail.clone());
            }
            self.error_msg = Some(detail.clone());
        }
        if self.analytics_source.as_ref().is_some_and(
            |source| matches!(source, StorageScanSource::Local { root } if affected(root)),
        ) {
            self.analytics_access.invalidate_partial();
            if let Some(scan) = self.analytics_scan.take() {
                scan.progress.cancel.store(true, Ordering::Release);
            }
            self.analytics_state = StorageRunState::Failed;
            self.analytics_tree = None;
            self.analytics_invalidate();
            self.analytics_issues.push(crate::analytics::ScanIssue {
                path: self
                    .analytics_source
                    .as_ref()
                    .map(|source| source.root().to_string())
                    .unwrap_or_default(),
                detail: detail.clone(),
            });
        }
        if self.reclaim_source.as_ref().is_some_and(
            |source| matches!(source, StorageScanSource::Local { root } if affected(root)),
        ) {
            self.cancel_reclaim_scan();
            self.reclaim_state = StorageRunState::Failed;
            self.reclaim_report = None;
            self.reclaim_selected.clear();
            self.reclaim_issues.push(detail);
        }
    }
}
