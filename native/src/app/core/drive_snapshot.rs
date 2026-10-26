//! Shared drive projection and identity-aware UI/index reconciliation.
//! OS inventory and event APIs remain behind platform_helpers.
use crate::app::prelude::*;
use crate::app::*;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[path = "drive_snapshot_apply.rs"]
mod apply;

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
    entries
        .iter()
        .filter(|entry| contains(&entry.root, path))
        .max_by_key(|entry| entry.root.len())
        .map(|entry| DriveBinding {
            root: entry.root.clone(),
            identity: entry.identity.clone(),
            disconnected: false,
        })
}

fn binding_available(binding: &DriveBinding, entries: &[DriveEntry]) -> bool {
    !binding.disconnected
        || entries.iter().any(|entry| {
            entry.root == binding.root
                && entry.identity.is_some()
                && entry.identity == binding.identity
        })
}

#[derive(Default)]
pub(crate) struct DriveChanges {
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

pub(crate) fn diff(old: &[DriveEntry], new: &[DriveEntry]) -> DriveChanges {
    let same = |a: &DriveEntry, b: &DriveEntry| {
        a.root == b.root && a.identity == b.identity && a.incarnation == b.incarnation
    };
    DriveChanges {
        removed: old
            .iter()
            .filter(|a| !new.iter().any(|b| same(a, b)))
            .map(|a| a.root.clone())
            .collect(),
        added: new
            .iter()
            .filter(|b| !old.iter().any(|a| same(a, b)))
            .map(|b| b.root.clone())
            .collect(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DriveConsumer {
    Analytics,
    Reclaim,
    Picker,
}

struct SourceBinding {
    source: String,
    binding: Option<DriveBinding>,
}

struct Delta {
    roots: Vec<DriveEntry>,
    generations: BTreeMap<String, u64>,
    rx: Receiver<IndexMsg>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Delta {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

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
        Self {
            enabled,
            initialized: false,
            index_enabled: false,
            monitor: None,
            entries: Vec::new(),
            focused: false,
            retry_at: Instant::now(),
            generations: BTreeMap::new(),
            pending: BTreeMap::new(),
            consumers: BTreeMap::new(),
            delta: None,
            index_obsolete: false,
        }
    }
}

/// Match a complete path segment; /media/a must not contain /media/ab.
pub(crate) fn contains(root: &str, path: &str) -> bool {
    let windows = root.as_bytes().get(1) == Some(&b':') || root.starts_with("\\\\");
    let (root, path) = if windows {
        (
            root.replace('\\', "/").to_lowercase(),
            path.replace('\\', "/").to_lowercase(),
        )
    } else {
        (root.to_string(), path.to_string())
    };
    let root = root.trim_end_matches('/');
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

impl App {
    pub(in crate::app) fn update_drives(&mut self, ctx: &egui::Context) {
        if !self.drive_state.enabled {
            return;
        }
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
            if let Some(monitor) = &self.drive_state.monitor {
                monitor.refresh();
            }
        }
        self.drive_state.focused = focused;
        let message = self
            .drive_state
            .monitor
            .as_ref()
            .and_then(|monitor| monitor.snapshots.try_recv().ok());
        if let Some(message) = message {
            match message {
                Ok(entries) => self.apply_drive_snapshot(entries),
                // Keep the last successful snapshot on inventory failure.
                Err(error) => {
                    if self.drive_state.retry_at <= Instant::now() {
                        self.push_app_error("Laufwerksbestand", error);
                        self.drive_state.retry_at =
                            Instant::now() + std::time::Duration::from_secs(30);
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

    /// Refresh must not adopt a different device mounted at the same path.
    /// Explicit navigation remains the user's way to select the new device.
    pub(in crate::app) fn disconnected_drive(&self, path: &str) -> bool {
        self.drive_binding.as_ref().is_some_and(|binding| {
            contains(&binding.root, path)
                && binding.disconnected
                && !self.drive_state.entries.iter().any(|entry| {
                    entry.root == binding.root
                        && entry.identity.is_some()
                        && entry.identity == binding.identity
                })
        })
    }

    pub(in crate::app) fn selecting_drive_root(&self, path: &str) -> bool {
        self.drive_state
            .entries
            .iter()
            .any(|entry| contains(&entry.root, path) && contains(path, &entry.root))
    }

    pub(in crate::app) fn bind_scan_drive(&mut self) {
        self.drive_binding = if self.remote.is_none() {
            binding_for(&self.drive_state.entries, &self.root_path)
        } else {
            None
        };
    }

    /// Called only for a deliberate local source selection, never for refresh.
    pub(in crate::app) fn accept_drive_consumer(&mut self, consumer: DriveConsumer, root: &str) {
        let binding = binding_for(&self.drive_state.entries, root);
        self.drive_state.consumers.insert(
            consumer,
            SourceBinding {
                source: root.into(),
                binding,
            },
        );
    }

    pub(in crate::app) fn drive_consumer_available(
        &mut self,
        consumer: DriveConsumer,
        root: &str,
        remote: bool,
    ) -> bool {
        if remote {
            self.drive_state.consumers.remove(&consumer);
            return true;
        }
        let current = self.drive_state.consumers.get(&consumer);
        // A changed child path is still inside the pinned device. Reject it
        // before binding the new path if that device was disconnected/replaced.
        if current
            .and_then(|state| state.binding.as_ref())
            .is_some_and(|binding| {
                contains(&binding.root, root)
                    && !binding_available(binding, &self.drive_state.entries)
            })
        {
            return false;
        }
        if current.is_none_or(|state| state.source != root || state.binding.is_none()) {
            self.accept_drive_consumer(consumer, root);
        }
        self.drive_state
            .consumers
            .get(&consumer)
            .and_then(|state| state.binding.as_ref())
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
        if self.index_building
            || self.drive_state.delta.is_some()
            || self.drive_state.pending.is_empty()
        {
            return;
        }
        // One changed root at a time: an unreadable medium must not discard
        // the successful index delta of an independent mounted root.
        let Some((_, entry)) = self.drive_state.pending.pop_first() else {
            return;
        };
        let roots = vec![entry];
        let paths = roots
            .iter()
            .map(|entry| PathBuf::from(&entry.root))
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let generations = self.drive_state.generations.clone();
        match super::drive_inventory::start_index_delta(paths, cancel.clone()) {
            Ok(rx) => {
                self.drive_state.delta = Some(Delta {
                    roots,
                    generations,
                    rx,
                    cancel,
                })
            }
            Err(error) => self.push_app_error("Laufwerksindex", error),
        }
    }

    fn drain_drive_index(&mut self) {
        let Some(delta) = self.drive_state.delta.take() else {
            return;
        };
        let message = match delta.rx.try_recv() {
            Ok(message) => message,
            Err(crossbeam_channel::TryRecvError::Empty) => {
                self.drive_state.delta = Some(delta);
                return;
            }
            Err(_) => {
                self.push_app_error("Laufwerksindex", "Indexworker wurde beendet");
                return;
            }
        };
        match message {
            IndexMsg::Complete(index) => {
                let valid: Vec<_> = delta
                    .roots
                    .iter()
                    .filter(|root| {
                        self.drive_state.entries.iter().any(|entry| {
                            entry.root == root.root
                                && entry.identity == root.identity
                                && entry.incarnation == root.incarnation
                        }) && delta.generations.get(&root.root)
                            == self.drive_state.generations.get(&root.root)
                    })
                    .collect();
                for path in index
                    .iter()
                    .filter(|path| valid.iter().any(|root| contains(&root.root, path)))
                {
                    match self.folder_index.try_insert(path.clone()) {
                        Ok(changed) => self.index_dirty |= changed,
                        Err(error) => {
                            self.push_app_error("Laufwerksindex", error.to_string());
                            break;
                        }
                    }
                }
                if !self.folder_search_query.is_empty() {
                    self.run_folder_search();
                }
            }
            IndexMsg::Failed(error) => self.push_app_error("Laufwerksindex", error),
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "batches_drive_task_tests.rs"]
mod batches_drive_task_tests;
