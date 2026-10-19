//! Paths of the device tree (DRV-M6, U17-14 and U17-16): which folder a
//! path names. A path is resolved from the left: the virtual folders (root,
//! `Räume`, a room, a device, `Verbindungen`), then the shared folder or
//! connection of a device, and the rest is a path inside that child's
//! engine. The virtual folders are read-only and need no connection.
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::sync::Mutex;

use super::device_tree::DeviceTree;
use super::device_tree_backend::{guard, unreachable};
use super::device_tree_children::ChildKey;
use super::device_tree_names::{same_name, sort_key, Layout, ROOMS_FOLDER};
use super::device_tree_types::ChildRoot;
use super::path_dialect::PathDialect;
use crate::vfs::VfsMeta;

/// The folder below a device that holds its connections.
pub(super) const CONNECTIONS: &str = "Verbindungen";

/// A folder above the shared folders.
pub(super) enum Dir {
    Root,
    Rooms,
    Room(String),
    Device(String),
    Connections(String),
}

/// A path inside one shared folder of a device.
pub(super) struct ChildRef {
    pub(super) child: ChildKey,
    pub(super) root: ChildRoot,
    pub(super) inner: Vec<String>,
}

impl ChildRef {
    /// The callback path the child's engine understands.
    pub(super) fn engine_path(&self, dialect: PathDialect) -> String {
        dialect.callback_path(&self.inner.join("/"))
    }

    /// The path below the engine's root, `/a/b`.
    pub(super) fn backend_path(&self) -> String {
        format!("/{}", self.inner.join("/"))
    }
}

/// Where a path leads.
pub(super) enum Routed {
    Virtual(Dir),
    /// The top folder of a shared folder: a directory that needs no
    /// connection to be named.
    Share(ChildRef),
    Inside(ChildRef),
}

enum Place {
    Dir(Dir),
    Device(String, usize),
    Missing,
}

pub(super) fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "Pfad nicht gefunden")
}

/// Every write below a virtual folder.
pub(super) fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "dieser Ordner des Gerätebaums ist schreibgeschützt",
    )
}

impl DeviceTree {
    /// The names of a callback path, validated by the rules of the dialect.
    pub(super) fn segments(&self, callback_path: &str) -> io::Result<Vec<String>> {
        let projected = self.projector.project(callback_path)?;
        Ok(projected
            .relative()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub(super) fn route(&self, callback_path: &str) -> io::Result<Routed> {
        let segments = self.segments(callback_path)?;
        self.route_segments(&segments, true)
    }

    /// The place a new name would be created at. A name that does not exist
    /// below a virtual folder is a write there: `PermissionDenied`, not
    /// `NotFound`.
    pub(super) fn route_new(&self, callback_path: &str) -> io::Result<Routed> {
        match self.route(callback_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let segments = self.segments(callback_path)?;
                let parent = &segments[..segments.len().saturating_sub(1)];
                match self.route_segments(parent, true) {
                    Ok(Routed::Virtual(_)) => Err(denied()),
                    _ => Err(error),
                }
            }
            other => other,
        }
    }

    /// `fetch` allows reading the shared folders of a device from the
    /// network; without it only what is known is used.
    pub(super) fn route_segments(&self, segments: &[String], fetch: bool) -> io::Result<Routed> {
        let Some(first) = segments.first() else {
            return Ok(Routed::Virtual(Dir::Root));
        };
        let dialect = self.dialect;
        let place = {
            let state = guard(&self.state);
            let layout = &state.layout;
            if let Some(entry) = layout
                .direct
                .iter()
                .find(|entry| same_name(dialect, &entry.name, first))
            {
                Place::Device(entry.key.clone(), 1)
            } else if same_name(dialect, first, ROOMS_FOLDER) && !layout.rooms.is_empty() {
                match segments.get(1) {
                    None => Place::Dir(Dir::Rooms),
                    Some(room_name) => match layout
                        .rooms
                        .iter()
                        .find(|room| same_name(dialect, &room.name, room_name))
                    {
                        None => Place::Missing,
                        Some(room) => match segments.get(2) {
                            None => Place::Dir(Dir::Room(room.id.clone())),
                            Some(device_name) => match room
                                .devices
                                .iter()
                                .find(|entry| same_name(dialect, &entry.name, device_name))
                            {
                                Some(entry) => Place::Device(entry.key.clone(), 3),
                                None => Place::Missing,
                            },
                        },
                    },
                }
            } else {
                Place::Missing
            }
        };
        match place {
            Place::Dir(dir) => Ok(Routed::Virtual(dir)),
            Place::Missing => Err(not_found()),
            Place::Device(key, from) => self.route_device(&key, &segments[from..], fetch),
        }
    }

    fn route_device(&self, key: &str, rest: &[String], fetch: bool) -> io::Result<Routed> {
        let Some(first) = rest.first() else {
            return Ok(Routed::Virtual(Dir::Device(key.to_string())));
        };
        let roots = self.roots_of(key, fetch)?;
        let dialect = self.dialect;
        let prefix = format!("{CONNECTIONS}/");
        let (root, skipped) = if same_name(dialect, first, CONNECTIONS) {
            if !roots.iter().any(|root| root.name.starts_with(&prefix)) {
                return Err(not_found());
            }
            let Some(connection) = rest.get(1) else {
                return Ok(Routed::Virtual(Dir::Connections(key.to_string())));
            };
            let found = roots.iter().find(|root| {
                root.name
                    .strip_prefix(&prefix)
                    .is_some_and(|name| same_name(dialect, name, connection))
            });
            (found.ok_or_else(not_found)?.clone(), 2)
        } else {
            let found = roots
                .iter()
                .find(|root| !root.name.contains('/') && same_name(dialect, &root.name, first));
            (found.ok_or_else(not_found)?.clone(), 1)
        };
        let child = ChildRef {
            child: ChildKey {
                key: key.to_string(),
                name: root.name.clone(),
            },
            inner: rest[skipped..].to_vec(),
            root,
        };
        Ok(if child.inner.is_empty() {
            Routed::Share(child)
        } else {
            Routed::Inside(child)
        })
    }

    /// The shared folders of a device. The first look reads them from the
    /// device; a device that cannot be reached is `HostUnreachable`, never
    /// an empty list. What was read stays known while the device is offline.
    pub(super) fn roots_of(&self, key: &str, fetch: bool) -> io::Result<Vec<ChildRoot>> {
        if let Some(roots) = guard(&self.roots).get(key) {
            return Ok(roots.clone());
        }
        if !fetch {
            return Err(not_found());
        }
        let (target, _) = self.target_of(key)?;
        let read = self.link.roots(&target).map_err(|_| unreachable())?;
        let roots = self.usable_roots(read);
        guard(&self.roots).insert(key.to_string(), roots.clone());
        Ok(roots)
    }

    /// The directory entry of a virtual folder.
    pub(super) fn dir_meta(&self, name: &str) -> VfsMeta {
        VfsMeta {
            name: name.to_string(),
            is_dir: true,
            mtime_ms: self.created_ms,
            btime_ms: self.created_ms,
            ..VfsMeta::default()
        }
    }

    /// The entries of a virtual folder, in display order.
    pub(super) fn list_virtual(&self, dir: &Dir) -> io::Result<Vec<VfsMeta>> {
        let mut names: Vec<String> = match dir {
            Dir::Root => {
                let state = guard(&self.state);
                let mut names: Vec<String> = state
                    .layout
                    .direct
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect();
                if !state.layout.rooms.is_empty() {
                    names.push(ROOMS_FOLDER.to_string());
                }
                names
            }
            Dir::Rooms => guard(&self.state)
                .layout
                .rooms
                .iter()
                .map(|room| room.name.clone())
                .collect(),
            Dir::Room(id) => guard(&self.state)
                .layout
                .rooms
                .iter()
                .find(|room| room.id == *id)
                .map(|room| {
                    room.devices
                        .iter()
                        .map(|entry| entry.name.clone())
                        .collect()
                })
                .ok_or_else(not_found)?,
            Dir::Device(key) => {
                let roots = self.roots_of(key, true)?;
                let mut names: Vec<String> = roots
                    .iter()
                    .filter(|root| !root.name.contains('/'))
                    .map(|root| root.name.clone())
                    .collect();
                if roots
                    .iter()
                    .any(|root| root.name.starts_with(&format!("{CONNECTIONS}/")))
                {
                    names.push(CONNECTIONS.to_string());
                }
                names
            }
            Dir::Connections(key) => self
                .roots_of(key, true)?
                .iter()
                .filter_map(|root| root.name.strip_prefix(&format!("{CONNECTIONS}/")))
                .map(str::to_string)
                .collect(),
        };
        names.sort_by_key(|name| sort_key(self.dialect, name));
        Ok(names.iter().map(|name| self.dir_meta(name)).collect())
    }
}

pub(super) fn drop_roots(roots: &Mutex<HashMap<String, Vec<ChildRoot>>>, gone: &[String]) {
    let mut roots = guard(roots);
    for key in gone {
        roots.remove(key);
    }
}

/// The folders of the layout as tree paths.
pub(super) fn layout_paths(layout: &Layout) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for entry in &layout.direct {
        paths.insert(format!("/{}", entry.name));
    }
    if !layout.rooms.is_empty() {
        paths.insert(format!("/{ROOMS_FOLDER}"));
    }
    for room in &layout.rooms {
        paths.insert(format!("/{ROOMS_FOLDER}/{}", room.name));
        for entry in &room.devices {
            paths.insert(format!("/{ROOMS_FOLDER}/{}/{}", room.name, entry.name));
        }
    }
    paths
}

/// The folders below a device folder: shared folders, connections and the
/// folder `Verbindungen`.
pub(super) fn root_paths(roots: &[ChildRoot]) -> BTreeSet<String> {
    let mut paths: BTreeSet<String> = roots.iter().map(|root| root.name.clone()).collect();
    if roots
        .iter()
        .any(|root| root.name.starts_with("Verbindungen/"))
    {
        paths.insert("Verbindungen".to_string());
    }
    paths
}
