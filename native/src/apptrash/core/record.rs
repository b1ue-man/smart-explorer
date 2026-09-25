//! Trash entry model, its metadata record (`<id>.json`) and the pure naming and
//! validation rules. Records live on shared storage, where other apps can
//! write, so every record is checked before it is trusted.
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One item in the app trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
    pub id: String,
    /// File or folder name, also the name inside the trash slot.
    pub name: String,
    /// Absolute path the item was deleted from.
    pub original: PathBuf,
    pub deleted_ms: i64,
    /// Bytes of the file, or of all files below a folder.
    pub size: u64,
    pub is_dir: bool,
}

pub(super) const RECORD_VERSION: u32 = 1;
pub(super) const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_ID_LEN: usize = 64;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Record {
    pub(super) version: u32,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) original: PathBuf,
    pub(super) deleted_ms: i64,
    pub(super) size: u64,
    pub(super) is_dir: bool,
}

impl Record {
    pub(super) fn from_entry(entry: &TrashEntry) -> Self {
        Self {
            version: RECORD_VERSION,
            id: entry.id.clone(),
            name: entry.name.clone(),
            original: entry.original.clone(),
            deleted_ms: entry.deleted_ms,
            size: entry.size,
            is_dir: entry.is_dir,
        }
    }

    pub(super) fn into_entry(self) -> TrashEntry {
        TrashEntry {
            id: self.id,
            name: self.name,
            original: self.original,
            deleted_ms: self.deleted_ms,
            size: self.size,
            is_dir: self.is_dir,
        }
    }

    /// Accepts a record only if it names itself by `id`, carries one plain
    /// name that ends its original path, and points back into `volume` (never
    /// into the volume's trash), so a planted record cannot redirect a restore.
    pub(super) fn is_trusted(&self, id: &str, volume: &Path, trash_root: &Path) -> bool {
        self.version == RECORD_VERSION
            && self.id == id
            && plain_name(&self.name)
            && self.original.is_absolute()
            && self.original.file_name().and_then(|name| name.to_str()) == Some(self.name.as_str())
            && self.original.starts_with(volume)
            && self.original != volume
            && !self.original.starts_with(trash_root)
            && self
                .original
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
    }
}

/// Trash ids are generated lowercase hex with dashes; anything else is refused
/// before it can become part of a path.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || byte == b'-')
}

pub(super) fn entry_id(deleted_ms: i64, random: u64) -> String {
    format!("{:x}-{random:016x}", deleted_ms.max(0))
}

/// Exactly one ordinary path component (no separator, `.` or `..`).
pub(super) fn plain_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(part)), None) if part == name
    )
}

/// `name` for the first attempt, then `stem (n).ext` for files and `name (n)`
/// for folders and dot files.
pub(super) fn numbered_name(name: &str, is_dir: bool, index: u32) -> String {
    if index <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => format!("{} ({index}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({index})"),
    }
}

/// The deepest volume that strictly contains `path` (all paths canonical).
pub(super) fn volume_for<'a>(volumes: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    volumes
        .iter()
        .filter(|volume| path.starts_with(volume) && path != volume.as_path())
        .max_by_key(|volume| volume.components().count())
}
