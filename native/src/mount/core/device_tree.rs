//! The device tree „Verbundene Geräte als Ordner“ (DRV-M6): every paired
//! device is a folder, every shared folder and connection of a device a
//! folder in it, and each of those is served by its own `MountEngine`
//! (`device_tree_children.rs`), opened at the first access.
//!
//! This file owns the tree itself: the device list the host feeds in, the
//! presence of devices (closing idle engines, publishing the journal when a
//! device comes back), and the recovery of children whose journal was never
//! published. Names are `device_tree_names.rs`, the path resolution and the
//! virtual folders `device_tree_routing.rs`, the frontend boundary
//! `device_tree_fs.rs`.
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use super::device_tree_backend::{guard, unreachable, LazyChildBackend};
use super::device_tree_children::{
    kind_id, load_book, store_book, ChildEngine, ChildFactory, ChildKey, Pin, Record,
};
use super::device_tree_names::{layout, Layout};
use super::device_tree_routing::{drop_roots, layout_paths, root_paths, ChildRef};
use super::device_tree_types::{ChildLink, ChildRoot, DeviceKind, DeviceRow, TreeConfig};
use super::metadata_cache::MetadataChange;
use super::mount_source::PeerMountTarget;
use super::path::PathProjector;
use super::path_dialect::PathDialect;
use super::types::{BackendRoot, DeleteToken, HandleId, MountId, MountMode};

/// What the host last told the tree about its devices.
#[derive(Default)]
pub(super) struct State {
    pub(super) rows: Vec<DeviceRow>,
    pub(super) layout: Layout,
    /// Presence at the last list, by device key.
    pub(super) online: HashMap<String, bool>,
    /// Devices that came online since the last `tick`.
    pub(super) came_online: Vec<String>,
    /// Changes of the folders above the shared folders, not yet drained.
    pub(super) changes: Vec<MetadataChange>,
}

pub struct DeviceTree {
    pub(super) id: MountId,
    pub(super) dialect: PathDialect,
    pub(super) projector: PathProjector,
    /// Time of the virtual folders: when this tree was made.
    pub(super) created_ms: i64,
    pub(super) link: Arc<dyn ChildLink>,
    pub(super) factory: ChildFactory,
    pub(super) state: Mutex<State>,
    /// The shared folders of the devices browsed so far, by device key.
    pub(super) roots: Mutex<HashMap<String, Vec<ChildRoot>>>,
    pub(super) children: Mutex<HashMap<ChildKey, Arc<ChildEngine>>>,
    gates: Mutex<HashMap<ChildKey, Arc<Mutex<()>>>>,
    book: Mutex<Vec<Record>>,
    /// Children whose journal could not be opened: path and reason.
    pub(super) broken: Mutex<Vec<(String, String)>>,
    pub(super) handles: Mutex<HashMap<u64, (Pin, HandleId)>>,
    pub(super) deletes: Mutex<HashMap<u64, (Pin, DeleteToken)>>,
    pub(super) next_handle: AtomicU64,
    pub(super) next_delete: AtomicU64,
    me: Weak<DeviceTree>,
}

impl DeviceTree {
    /// A tree over `link`. Children whose journal still holds changes are
    /// opened again, without any network access.
    pub fn new(
        config: TreeConfig,
        link: Arc<dyn ChildLink>,
        spool_root: &Path,
        dialect: PathDialect,
    ) -> io::Result<Arc<Self>> {
        let records = load_book(spool_root)?;
        let created_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
            .unwrap_or_default();
        let factory = ChildFactory {
            spool_root: spool_root.to_path_buf(),
            dialect,
            metadata: config.metadata,
            cache: config.cache,
            space: config.space,
        };
        let tree = Arc::new_cyclic(|me| Self {
            id: config.id,
            dialect,
            projector: PathProjector::with_dialect(BackendRoot::top(), dialect),
            created_ms,
            link,
            factory,
            state: Mutex::new(State::default()),
            roots: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
            book: Mutex::new(Vec::new()),
            broken: Mutex::new(Vec::new()),
            handles: Mutex::new(HashMap::new()),
            deletes: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            next_delete: AtomicU64::new(1),
            me: me.clone(),
        });
        tree.recover(records);
        Ok(tree)
    }

    /// Replaces the device list: new and removed folders become changes the
    /// frontend learns about, a device that came online is noted for the
    /// next `tick`.
    pub fn set_devices(&self, mut rows: Vec<DeviceRow>) {
        rows.sort_by(|left, right| left.key.cmp(&right.key));
        let mut state = guard(&self.state);
        let next = layout(&rows, self.dialect);
        let before = layout_paths(&state.layout);
        let after = layout_paths(&next);
        for path in after.difference(&before) {
            state.changes.push(MetadataChange::Created {
                path: path.clone(),
                is_directory: true,
            });
        }
        for path in before.difference(&after) {
            state.changes.push(MetadataChange::Deleted {
                path: path.clone(),
                is_directory: true,
            });
        }
        let mut online = HashMap::new();
        for row in &rows {
            if row.online && state.online.get(&row.key) != Some(&true) {
                state.came_online.push(row.key.clone());
            }
            online.insert(row.key.clone(), row.online);
        }
        let gone: Vec<String> = state
            .online
            .keys()
            .filter(|key| !online.contains_key(*key))
            .cloned()
            .collect();
        state.online = online;
        state.rows = rows;
        state.layout = next;
        drop(state);
        drop_roots(&self.roots, &gone);
    }

    /// One round of the host's control loop: publishes the journal of a
    /// device that came back, closes the engines of devices that are gone
    /// or offline as soon as they hold neither an open file nor a journal
    /// entry, and remembers how many journal entries each child holds.
    pub fn tick(&self) {
        let online = std::mem::take(&mut guard(&self.state).came_online);
        for key in online {
            self.on_online(&key);
        }
        self.sweep();
    }

    /// The path segments of the folder of a device.
    pub fn folder_of(&self, key: &str) -> Option<Vec<String>> {
        guard(&self.state).layout.folder_of(key)
    }

    /// The folder of a child as a tree path, `/Gerät/Freigabe`.
    pub(super) fn child_prefix(&self, key: &ChildKey) -> Option<String> {
        let folder = self.folder_of(&key.key)?;
        Some(format!("/{}/{}", folder.join("/"), key.name))
    }

    pub(super) fn target_of(&self, key: &str) -> io::Result<(PeerMountTarget, DeviceKind)> {
        guard(&self.state)
            .rows
            .iter()
            .find(|row| row.key == key)
            .map(|row| (row.target.clone(), row.kind))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unbekanntes Gerät"))
    }

    pub(super) fn open_children(&self) -> Vec<(ChildKey, Arc<ChildEngine>)> {
        guard(&self.children)
            .iter()
            .map(|(key, child)| (key.clone(), Arc::clone(child)))
            .collect()
    }

    /// The engine of a shared folder; the first access opens it. A device
    /// that cannot be reached is `HostUnreachable`, never an empty folder.
    pub(super) fn ensure_child(&self, child: &ChildRef) -> io::Result<Pin> {
        if let Some(open) = self.pin_open(&child.child) {
            return Ok(open);
        }
        let gate = Arc::clone(guard(&self.gates).entry(child.child.clone()).or_default());
        let _opening = guard(&gate);
        if let Some(open) = self.pin_open(&child.child) {
            return Ok(open);
        }
        let (target, kind) = self.target_of(&child.child.key)?;
        let connection = self
            .link
            .connect(&target, &child.root.root)
            .map_err(|_| unreachable())?;
        let mode = if kind == DeviceKind::Direct && connection.writable {
            MountMode::ReadWrite
        } else {
            MountMode::ReadOnly
        };
        let id = kind_id(&child.child.key, child.root.root.as_str());
        let lease = self
            .link
            .acquire_lease(&self.factory.spool_root, &MountId::parse(&id)?)?;
        let backend = LazyChildBackend::seeded(
            Arc::clone(&self.link),
            target.clone(),
            child.root.root.clone(),
            connection,
        );
        let opened = self.factory.open(&id, mode, backend, lease)?;
        self.remember(Record {
            id,
            key: child.child.key.clone(),
            target,
            root: child.root.root.as_str().to_string(),
            name: child.child.name.clone(),
            mode: mode.into(),
            pending: u32::try_from(opened.pending()).unwrap_or(u32::MAX),
        });
        let mut children = guard(&self.children);
        let pin = Pin::new(&opened);
        children.insert(child.child.clone(), opened);
        Ok(pin)
    }

    /// A hold on the engine of a child that is open now.
    pub(super) fn pin_open(&self, key: &ChildKey) -> Option<Pin> {
        guard(&self.children).get(key).map(Pin::new)
    }

    /// The engines of the children of devices that are reachable.
    fn on_online(&self, key: &str) {
        let key = key.to_string();
        self.background("devices-retry", move |tree| {
            tree.refresh_roots(&key);
            let waiting: Vec<_> = tree
                .open_children()
                .into_iter()
                .filter(|(child, engine)| child.key == key && engine.pending() > 0)
                .collect();
            for (_, engine) in waiting {
                if let Err(error) = engine.retry() {
                    eprintln!(
                        "Gerätebaum: Änderungen von {key} sind noch nicht gespeichert: {error}"
                    );
                }
            }
        });
    }

    fn background(&self, name: &str, work: impl FnOnce(Arc<DeviceTree>) + Send + 'static) {
        if let Some(tree) = self.me.upgrade() {
            let _ = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || work(tree));
        }
    }

    /// Reads the shared folders of a device again, when they were read
    /// before; added and removed folders become changes.
    pub(super) fn refresh_roots(&self, key: &str) {
        let Some(old) = guard(&self.roots).get(key).cloned() else {
            return;
        };
        let Ok((target, _)) = self.target_of(key) else {
            return;
        };
        let Ok(read) = self.link.roots(&target) else {
            return;
        };
        let new = self.usable_roots(read);
        if new == old {
            return;
        }
        let prefix = self
            .folder_of(key)
            .map(|folder| format!("/{}", folder.join("/")));
        if let Some(prefix) = prefix {
            let (before, after) = (root_paths(&old), root_paths(&new));
            let mut state = guard(&self.state);
            for path in after.difference(&before) {
                state.changes.push(MetadataChange::Created {
                    path: format!("{prefix}/{path}"),
                    is_directory: true,
                });
            }
            for path in before.difference(&after) {
                state.changes.push(MetadataChange::Deleted {
                    path: format!("{prefix}/{path}"),
                    is_directory: true,
                });
            }
        }
        guard(&self.roots).insert(key.to_string(), new);
    }

    /// Only folders every segment of which the dialect can name.
    pub(super) fn usable_roots(&self, roots: Vec<ChildRoot>) -> Vec<ChildRoot> {
        roots
            .into_iter()
            .filter(|root| {
                root.name
                    .split('/')
                    .all(|segment| self.dialect.validate_component(segment).is_ok())
            })
            .collect()
    }

    fn sweep(&self) {
        let rows = guard(&self.state).rows.clone();
        for (key, child) in self.open_children() {
            let row = rows.iter().find(|row| row.key == key.key);
            let present = row.is_some_and(|row| row.online);
            let wanted = match (row, child.backend.observed_writable()) {
                (Some(row), Some(true)) if row.kind == DeviceKind::Direct => MountMode::ReadWrite,
                (Some(_), Some(_)) => MountMode::ReadOnly,
                _ => child.mode,
            };
            if (!present || wanted != child.mode) && child.idle() {
                self.close_child(&key, &child);
            }
        }
        self.save_pending();
    }

    /// Ends an idle engine; its cache lease goes with the last reference.
    fn close_child(&self, key: &ChildKey, child: &Arc<ChildEngine>) {
        let mut children = guard(&self.children);
        if children
            .get(key)
            .is_some_and(|held| Arc::ptr_eq(held, child))
            && child.idle()
        {
            children.remove(key);
            drop(children);
            self.forget(&key.key, &key.name);
        }
    }

    fn remember(&self, record: Record) {
        let mut book = guard(&self.book);
        book.retain(|other| other.id != record.id);
        book.push(record);
        self.write_book(&book);
    }

    /// A child that was closed idle has an empty journal and needs no entry.
    fn forget(&self, key: &str, name: &str) {
        let mut book = guard(&self.book);
        let before = book.len();
        book.retain(|record| !(record.key == key && record.name == name));
        if book.len() != before {
            self.write_book(&book);
        }
    }

    fn save_pending(&self) {
        let open = self.open_children();
        let mut book = guard(&self.book);
        let mut changed = false;
        for record in book.iter_mut() {
            if let Some((_, child)) = open
                .iter()
                .find(|(key, _)| key.key == record.key && key.name == record.name)
            {
                let pending = u32::try_from(child.pending()).unwrap_or(u32::MAX);
                if record.pending != pending {
                    record.pending = pending;
                    changed = true;
                }
            }
        }
        if changed {
            self.write_book(&book);
        }
    }

    fn write_book(&self, records: &[Record]) {
        if let Err(error) = store_book(&self.factory.spool_root, records) {
            eprintln!("Gerätebaum: children.json konnte nicht geschrieben werden: {error}");
        }
    }

    /// Opens the children of an earlier run whose journal is not empty.
    fn recover(&self, records: Vec<Record>) {
        let mut kept = Vec::new();
        for mut record in records {
            match self.reopen(&record) {
                Reopened::Empty => {}
                Reopened::Open(pending) => {
                    record.pending = pending;
                    kept.push(record);
                }
                Reopened::Broken(reason) => {
                    guard(&self.broken).push((format!("/{}/{}", record.key, record.name), reason));
                    kept.push(record);
                }
            }
        }
        *guard(&self.book) = kept.clone();
        self.write_book(&kept);
    }

    fn reopen(&self, record: &Record) -> Reopened {
        let (Ok(id), Ok(root)) = (MountId::parse(&record.id), BackendRoot::parse(&record.root))
        else {
            return Reopened::Broken("ungültiger Eintrag in children.json".into());
        };
        let spool_root = &self.factory.spool_root;
        let lease = match self.link.acquire_lease(spool_root, &id) {
            Ok(lease) => lease,
            Err(error) => return Reopened::Broken(format!("Zwischenspeicher gesperrt: {error}")),
        };
        if matches!(super::spool::audit_pending(spool_root, &id), Ok(0)) {
            return Reopened::Empty;
        }
        let backend = LazyChildBackend::new(Arc::clone(&self.link), record.target.clone(), root);
        match self
            .factory
            .open(&record.id, record.mode.into(), backend, lease)
        {
            Ok(child) => {
                let pending = u32::try_from(child.pending()).unwrap_or(u32::MAX);
                let key = ChildKey {
                    key: record.key.clone(),
                    name: record.name.clone(),
                };
                guard(&self.children).insert(key, child);
                Reopened::Open(pending)
            }
            Err(error) => Reopened::Broken(format!(
                "Zwischenspeicher konnte nicht geöffnet werden: {error}"
            )),
        }
    }
}

enum Reopened {
    Empty,
    Open(u32),
    Broken(String),
}

// The fixture also serves the Dokany harness of the Windows host.
#[cfg(test)]
#[path = "device_tree_test_support.rs"]
pub(in crate::mount) mod support;
#[cfg(test)]
#[path = "device_tree_tests.rs"]
mod tests;
