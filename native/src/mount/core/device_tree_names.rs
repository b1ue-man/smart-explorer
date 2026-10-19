//! Folder names of the device tree (DRV-M6, U17-14): the key of a device,
//! the rules that turn display names into folder names, and the layout of
//! the tree (direct devices at the top, rooms below `Räume`).
//!
//! Names are deterministic and independent of the order of the input:
//! 1. forbidden characters become `_`, names are cut to the length of the
//!    dialect, an empty result is `Gerät`;
//! 2. a name the dialect rejects becomes `Gerät <6 hex>` (SHA-256 of the key);
//! 3. a device whose name equals `Räume` becomes `Räume (Gerät)`;
//! 4. names equal under the dialect all get ` (<6 hex>)`.
use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::device_tree_types::{DeviceKind, DeviceRow, RoomRef};
use super::mount_source::PeerMountTarget;
use super::path_dialect::{PathDialect, POSIX_NAME_MAX_BYTES};
use super::windows_case::windows_ordinal_key;

/// The one drive of the device tree (a valid `MountId`).
pub const DEVICES_MOUNT_ID: &str = "se-devices";
/// The folder that holds the rooms.
pub const ROOMS_FOLDER: &str = "Räume";
const ROOMS_CLASH: &str = "Räume (Gerät)";
const FALLBACK: &str = "Gerät";
const WINDOWS_NAME_UNITS: usize = 255;

/// `direct~<contact_id>` or `room~<room_id>~<device_id>`; every byte outside
/// `[A-Za-z0-9._-]` is `%XX` in capital hex (contract V4).
pub fn device_key_of(target: &PeerMountTarget) -> String {
    match target {
        PeerMountTarget::Direct { contact_id } => format!("direct~{}", escape(contact_id)),
        PeerMountTarget::RoomDevice { room_id, device_id } => {
            format!("room~{}~{}", escape(room_id), escape(device_id))
        }
    }
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    escaped
}

/// One folder of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub key: String,
}

/// A room with its member devices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomFolder {
    pub id: String,
    pub name: String,
    pub devices: Vec<Entry>,
}

/// The folders above the shared folders, in display order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub direct: Vec<Entry>,
    pub rooms: Vec<RoomFolder>,
}

impl Layout {
    /// The path segments of the folder of a device.
    pub fn folder_of(&self, key: &str) -> Option<Vec<String>> {
        if let Some(entry) = self.direct.iter().find(|entry| entry.key == key) {
            return Some(vec![entry.name.clone()]);
        }
        self.rooms.iter().find_map(|room| {
            room.devices
                .iter()
                .find(|entry| entry.key == key)
                .map(|entry| {
                    vec![
                        ROOMS_FOLDER.to_string(),
                        room.name.clone(),
                        entry.name.clone(),
                    ]
                })
        })
    }
}

/// The layout of `rows` under the name rules of `dialect`.
pub fn layout(rows: &[DeviceRow], dialect: PathDialect) -> Layout {
    let mut direct_rows: Vec<&DeviceRow> = rows
        .iter()
        .filter(|row| row.kind == DeviceKind::Direct)
        .collect();
    direct_rows.sort_by(|left, right| left.key.cmp(&right.key));
    let names = assign(&pairs(&direct_rows), dialect, true);
    let mut direct: Vec<Entry> = direct_rows
        .iter()
        .zip(names)
        .map(|(row, name)| Entry {
            name,
            key: row.key.clone(),
        })
        .collect();
    sort_entries(&mut direct, dialect);

    let mut by_room: BTreeMap<&str, (&RoomRef, Vec<&DeviceRow>)> = BTreeMap::new();
    let mut room_rows: Vec<&DeviceRow> = rows
        .iter()
        .filter(|row| row.kind == DeviceKind::Room)
        .collect();
    room_rows.sort_by(|left, right| left.key.cmp(&right.key));
    for row in room_rows {
        if let Some(room) = row.room.as_ref() {
            by_room
                .entry(room.id.as_str())
                .or_insert_with(|| (room, Vec::new()))
                .1
                .push(row);
        }
    }
    let room_pairs: Vec<(String, String)> = by_room
        .iter()
        .map(|(id, (room, _))| (id.to_string(), room.name.clone()))
        .collect();
    let room_names = assign(&room_pairs, dialect, false);
    let mut rooms: Vec<RoomFolder> = by_room
        .into_iter()
        .zip(room_names)
        .map(|((id, (_, members)), name)| {
            let names = assign(&pairs(&members), dialect, false);
            let mut devices: Vec<Entry> = members
                .iter()
                .zip(names)
                .map(|(row, name)| Entry {
                    name,
                    key: row.key.clone(),
                })
                .collect();
            sort_entries(&mut devices, dialect);
            RoomFolder {
                id: id.to_string(),
                name,
                devices,
            }
        })
        .collect();
    rooms.sort_by(|left, right| {
        sort_key(dialect, &left.name)
            .cmp(&sort_key(dialect, &right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Layout { direct, rooms }
}

/// Key of a device to the name of its folder (inside its room for room
/// devices).
pub fn folder_names(rows: &[DeviceRow], dialect: PathDialect) -> BTreeMap<String, String> {
    let layout = layout(rows, dialect);
    let mut names = BTreeMap::new();
    for entry in layout
        .direct
        .iter()
        .chain(layout.rooms.iter().flat_map(|room| room.devices.iter()))
    {
        names.insert(entry.key.clone(), entry.name.clone());
    }
    names
}

/// Id of a room to the name of its folder below `Räume`.
pub fn room_folder_names(rooms: &[RoomRef], dialect: PathDialect) -> BTreeMap<String, String> {
    let mut unique: BTreeMap<&str, &RoomRef> = BTreeMap::new();
    for room in rooms {
        unique.entry(room.id.as_str()).or_insert(room);
    }
    let items: Vec<(String, String)> = unique
        .values()
        .map(|room| (room.id.clone(), room.name.clone()))
        .collect();
    items
        .iter()
        .map(|(id, _)| id.clone())
        .zip(assign(&items, dialect, false))
        .collect()
}

/// Whether two names are the same name under `dialect`.
pub fn same_name(dialect: PathDialect, left: &str, right: &str) -> bool {
    sort_key(dialect, left) == sort_key(dialect, right)
}

fn pairs(rows: &[&DeviceRow]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| (row.key.clone(), row.name.clone()))
        .collect()
}

fn sort_entries(entries: &mut [Entry], dialect: PathDialect) {
    entries.sort_by(|left, right| {
        sort_key(dialect, &left.name)
            .cmp(&sort_key(dialect, &right.name))
            .then_with(|| left.key.cmp(&right.key))
    });
}

/// Windows compares names by their ordinal upper-case form, POSIX by bytes.
pub fn sort_key(dialect: PathDialect, name: &str) -> String {
    match dialect {
        PathDialect::Windows => windows_ordinal_key(name),
        PathDialect::Posix => name.to_string(),
    }
}

/// Folder names for `items` (id for the hash, display name), in the order of
/// `items`. `devices_level` reserves the name of the rooms folder.
fn assign(items: &[(String, String)], dialect: PathDialect, devices_level: bool) -> Vec<String> {
    let mut names: Vec<String> = items
        .iter()
        .map(|(id, raw)| {
            let mut name = sanitize(raw, dialect);
            if dialect.validate_component(&name).is_err() {
                name = format!("{FALLBACK} {}", short(id));
            }
            if devices_level && same_name(dialect, &name, ROOMS_FOLDER) {
                name = ROOMS_CLASH.to_string();
            }
            name
        })
        .collect();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, name) in names.iter().enumerate() {
        groups
            .entry(sort_key(dialect, name))
            .or_default()
            .push(index);
    }
    for indexes in groups.values().filter(|indexes| indexes.len() > 1) {
        for &index in indexes {
            let suffix = format!(" ({})", short(&items[index].0));
            let base = cut(&names[index], dialect, suffix.len());
            names[index] = format!("{base}{suffix}");
        }
    }
    names
}

/// Forbidden characters, length, trailing blank or dot and DOS device names.
fn sanitize(raw: &str, dialect: PathDialect) -> String {
    let mut name: String = raw
        .chars()
        .map(|character| {
            if character == '\0' || character.is_control() || forbidden(character, dialect) {
                '_'
            } else {
                character
            }
        })
        .collect();
    if dialect == PathDialect::Posix && matches!(name.as_str(), "." | "..") {
        name = "_".to_string();
    }
    name = cut(&name, dialect, 0);
    if dialect == PathDialect::Windows {
        if name.ends_with(' ') || name.ends_with('.') {
            name.pop();
            name.push('_');
        }
        if is_dos_device(&name) {
            name = cut(&name, dialect, 1);
            name.push('_');
        }
    }
    if name.is_empty() {
        FALLBACK.to_string()
    } else {
        name
    }
}

fn forbidden(character: char, dialect: PathDialect) -> bool {
    match dialect {
        PathDialect::Windows => "<>:\"/\\|?*".contains(character),
        PathDialect::Posix => character == '/',
    }
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`, with any
/// extension after a dot and in any letter case.
fn is_dos_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

/// `name` cut at a character boundary to the length of the dialect minus
/// `reserve` units (UTF-16 units under Windows, bytes under POSIX).
fn cut(name: &str, dialect: PathDialect, reserve: usize) -> String {
    let limit = match dialect {
        PathDialect::Windows => WINDOWS_NAME_UNITS,
        PathDialect::Posix => POSIX_NAME_MAX_BYTES,
    }
    .saturating_sub(reserve);
    let mut used = 0;
    let mut cut = String::new();
    for character in name.chars() {
        let units = match dialect {
            PathDialect::Windows => character.len_utf16(),
            PathDialect::Posix => character.len_utf8(),
        };
        if used + units > limit {
            break;
        }
        used += units;
        cut.push(character);
    }
    cut
}

/// The first six hex digits of the SHA-256 of `id`, in lower case.
fn short(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    format!("{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2])
}
