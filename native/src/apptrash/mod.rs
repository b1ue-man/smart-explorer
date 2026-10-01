//! App trash for platforms without a system recycle bin for arbitrary files
//! (Android). Deleted items move by rename into
//! `<volume>/.SmartExplorer-Papierkorb/<id>/<name>` on the same storage volume,
//! next to an `<id>.json` record, and can be restored or purged later.
//!
//! Platform-neutral and inert until an embedding host calls `set_volumes`; the
//! desktop builds never do, so their recycle-bin behavior and scans stay as
//! they were.
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

#[path = "core/record.rs"]
mod record;
#[path = "os/shared/store.rs"]
mod store;
// The app trash only runs on Android; its filesystem tests use POSIX paths
// (a Windows canonical `\\?\` prefix is never a trusted record origin).
#[cfg(all(test, unix))]
#[path = "os/shared/store_tests.rs"]
mod store_tests;

pub use record::TrashEntry;

/// Folder name of the trash at every volume root.
pub const TRASH_DIR_NAME: &str = ".SmartExplorer-Papierkorb";

static VOLUMES: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Scans, indexes, transfers and sync walks skip this name as a protected
/// omission, but only while the app trash is active (volumes set).
pub fn excluded_name(name: &str) -> bool {
    is_excluded(name, || {
        !VOLUMES
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    })
}

/// The name check runs first, so the hot scan path takes no lock.
fn is_excluded(name: &str, trash_active: impl FnOnce() -> bool) -> bool {
    name == TRASH_DIR_NAME && trash_active()
}

/// Whether the entries inside `dir` are other apps' private storage
/// (`<volume>/Android/data`, `<volume>/Android/obb`): Android lists the package
/// folders and files such as `.nomedia` there but refuses to open them even with
/// all-files access. Sync walks omit them like the trash, as protected
/// omissions. Inert while no volumes are set.
pub fn hidden_app_folders_in(dir: &str) -> bool {
    in_hidden_app_parent(dir, volumes)
}

/// The suffix check runs first, so the hot walk path takes no lock.
fn in_hidden_app_parent(dir: &str, volumes: impl FnOnce() -> Vec<PathBuf>) -> bool {
    let dir = dir.trim_end_matches('/');
    let Some(volume) = dir
        .strip_suffix("/Android/data")
        .or_else(|| dir.strip_suffix("/Android/obb"))
    else {
        return false;
    };
    volumes().iter().any(|root| {
        root.to_str()
            .is_some_and(|root| root.trim_end_matches('/') == volume)
    })
}

/// Folders below `<volume>/Android` that hold other apps' private storage.
const APP_PRIVATE_DIRS: [&str; 2] = ["data", "obb"];
/// Aliases of the primary (emulated) volume that a walk root may start at.
const PRIMARY_ALIASES: [&str; 2] = ["/sdcard", "/storage/self/primary"];

/// Other apps' private storage as one walk meets it: `<volume>/Android/data`
/// and `<volume>/Android/obb` with everything below, in the walk's own path
/// form (a walk may start at an alias such as `/sdcard`). Since Android 11 no
/// app may open other apps' folders there, also with all-files access, so a
/// walk counts what it cannot read inside as a protected omission instead of a
/// read error; readable parts (the app's own folder) are still walked.
/// Empty while no volumes are set, so desktop walks stay as they were.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtectedAreas {
    areas: Vec<PathBuf>,
}

impl ProtectedAreas {
    pub fn for_walk(root: &Path) -> Self {
        Self::for_walk_in(root, &volumes())
    }

    /// `for_walk` against an explicit volume list instead of the global one.
    pub fn for_walk_in(root: &Path, volumes: &[PathBuf]) -> Self {
        let Some(canonical) = canonical_form(root, volumes) else {
            return Self::default();
        };
        let mut areas: Vec<PathBuf> = Vec::new();
        for volume in volumes {
            let volume = std::fs::canonicalize(volume).unwrap_or_else(|_| lexical(volume));
            for name in APP_PRIVATE_DIRS {
                let area = volume.join("Android").join(name);
                let walk_form = if let Some(rest) = rest_after(&area, &canonical) {
                    join_rest(root, &rest)
                } else if starts_with(&canonical, &area) {
                    // The root lies inside the area: the area is the root
                    // without the components that `canonical` has below it.
                    let below = canonical.components().count() - area.components().count();
                    let keep = root.components().count().saturating_sub(below);
                    root.components().take(keep).collect()
                } else {
                    continue;
                };
                if !areas.iter().any(|known| same_path(known, &walk_form)) {
                    areas.push(walk_form);
                }
            }
        }
        Self { areas }
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    pub fn areas(&self) -> &[PathBuf] {
        &self.areas
    }

    /// The protected area that contains `path` (the area itself or below it).
    pub fn area_of(&self, path: &Path) -> Option<&Path> {
        self.areas
            .iter()
            .find(|area| starts_with(path, area))
            .map(PathBuf::as_path)
    }

    /// Whether `path` is exactly one of the areas (the name is compared first,
    /// so the hot walk path stays cheap).
    pub fn is_area(&self, path: &Path) -> bool {
        let Some(name) = path.file_name() else {
            return false;
        };
        self.areas.iter().any(|area| {
            area.file_name()
                .is_some_and(|own| own.eq_ignore_ascii_case(name))
                && same_path(area, path)
        })
    }

    /// The areas that are entries of `dir` (`dir` is a `<volume>/Android`).
    pub fn children_of<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = &'a Path> + 'a {
        self.areas
            .iter()
            .filter(move |area| area.parent().is_some_and(|parent| same_path(parent, dir)))
            .map(PathBuf::as_path)
    }
}

/// Where a walk root lies on its storage volume: the volume as registered and
/// the root's own segments below it (empty for the volume root itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumePlace {
    pub volume: PathBuf,
    pub below: Vec<String>,
}

pub fn volume_place(root: &Path) -> Option<VolumePlace> {
    volume_place_in(root, &volumes())
}

/// `volume_place` against an explicit volume list instead of the global one.
pub fn volume_place_in(root: &Path, volumes: &[PathBuf]) -> Option<VolumePlace> {
    let canonical = canonical_form(root, volumes)?;
    volumes
        .iter()
        .filter_map(|volume| {
            let resolved = std::fs::canonicalize(volume).unwrap_or_else(|_| lexical(volume));
            let rest = rest_after(&canonical, &resolved)?;
            Some((resolved.components().count(), volume, rest))
        })
        .max_by_key(|(depth, _, _)| *depth)
        .map(|(_, volume, rest)| VolumePlace {
            volume: volume.clone(),
            below: rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect(),
        })
}

/// The real location of `root`: the longest existing ancestor resolved by
/// the file system, the missing rest appended, and the primary volume's
/// aliases mapped when nothing resolved them. `None` for relative roots or
/// while no volumes are set.
fn canonical_form(root: &Path, volumes: &[PathBuf]) -> Option<PathBuf> {
    if volumes.is_empty() {
        return None;
    }
    let lexical = lexical(root);
    if !lexical.has_root() {
        return None;
    }
    let mut existing = lexical.as_path();
    let mut missing = Vec::new();
    let mut resolved = loop {
        if let Ok(found) = std::fs::canonicalize(existing) {
            break found;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => break existing.to_path_buf(),
        }
    };
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    Some(unalias(resolved, volumes))
}

fn unalias(path: PathBuf, volumes: &[PathBuf]) -> PathBuf {
    let primary = volumes.iter().find(|volume| {
        rest_after(volume, Path::new("/storage/emulated"))
            .is_some_and(|rest| rest.components().count() == 1)
    });
    let Some(primary) = primary else {
        return path;
    };
    for alias in PRIMARY_ALIASES {
        if let Some(rest) = rest_after(&path, Path::new(alias)) {
            return join_rest(primary, &rest);
        }
    }
    path
}

/// `.` dropped and `..` folded without touching the file system.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn join_rest(base: &Path, rest: &Path) -> PathBuf {
    if rest.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rest)
    }
}

/// Android's shared storage compares names without regard to ASCII case.
fn same_component(left: Component<'_>, right: Component<'_>) -> bool {
    left.as_os_str()
        .as_encoded_bytes()
        .eq_ignore_ascii_case(right.as_os_str().as_encoded_bytes())
}

/// The components of `path` after `prefix`, or `None` when `path` does not
/// start with `prefix`.
fn rest_after(path: &Path, prefix: &Path) -> Option<PathBuf> {
    let mut parts = path.components();
    for want in prefix.components() {
        if !parts.next().is_some_and(|have| same_component(have, want)) {
            return None;
        }
    }
    Some(parts.as_path().to_path_buf())
}

fn starts_with(path: &Path, prefix: &Path) -> bool {
    let mut parts = path.components();
    prefix
        .components()
        .all(|want| parts.next().is_some_and(|have| same_component(have, want)))
}

fn same_path(left: &Path, right: &Path) -> bool {
    let (mut left, mut right) = (left.components(), right.components());
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if same_component(a, b) => {}
            _ => return false,
        }
    }
}

/// Replaces the storage volumes (absolute roots) that carry an app trash.
pub fn set_volumes(volumes: Vec<PathBuf>) {
    *VOLUMES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = volumes;
}

/// Moves `path` into the trash of the volume that contains it. Never deletes:
/// if the move fails, the item stays where it was and the error is returned.
pub fn move_to_trash(path: &Path) -> io::Result<TrashEntry> {
    store::move_to_trash_in(&volumes(), path)
}

/// All entries of all volumes, newest first.
pub fn list() -> io::Result<Vec<TrashEntry>> {
    store::list_in(&volumes())
}

/// Moves an entry back to its original folder (recreated if needed) and
/// returns the restored path; an occupied name becomes `Name (2)` etc.
pub fn restore(id: &str) -> io::Result<PathBuf> {
    store::restore_in(&volumes(), id)
}

/// Deletes an entry permanently.
pub fn delete(id: &str) -> io::Result<()> {
    store::delete_in(&volumes(), id)
}

/// Deletes every entry older than `days` days and returns how many went.
pub fn purge_older_than(days: u32) -> io::Result<usize> {
    store::purge_older_than_in(&volumes(), days, store::now_ms())
}

fn volumes() -> Vec<PathBuf> {
    VOLUMES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}
