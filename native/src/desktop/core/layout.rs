//! Arrangement of the surface per monitor (C5.4 `layout/<Monitorschlüssel>.json`,
//! C5.5, Ä27). The shell is the only writer; this module plans every change,
//! the os layer writes the files atomically and deletes the files of monitors
//! that [`DesktopLayout::adopt_removed_monitors`] reports.
//!
//! Item IDs hold device keys, room IDs, vault site IDs (`vault:<site-id>`)
//! and desktop file names; never a path or a name from inside a vault (S5-V5).
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::items::SystemItem;

pub(crate) const LAYOUT_SCHEMA: u32 = 1;
/// Grid rows assumed until the surface reports its work area.
pub(crate) const DEFAULT_ROWS: u32 = 8;
const MAX_MONITOR_KEY: usize = 64;

/// One item of the surface.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) enum ItemId {
    /// `direct~<contact_id>` or `room~<room_id>~<device_id>` (V4).
    Device { key: String },
    /// `room:<room_id>`: the group of a room.
    Room { room_id: String },
    /// `vault:<site-id>`.
    Vault { site: String },
    /// `file:<name>` in the user's desktop folder.
    File { name: String },
    /// `public-file:<name>` in the all-users desktop folder (Windows).
    PublicFile { name: String },
    /// `system:recycle_bin`, `system:this_pc` (Windows).
    System(SystemItem),
    /// `screens:all`, the tile "Alle Bildschirme".
    AllScreens,
    /// `action:add_device` (empty state).
    AddDevice,
    /// `action:join_room` (empty state).
    JoinRoom,
}

impl ItemId {
    pub(crate) fn encode(&self) -> String {
        match self {
            ItemId::Device { key } => key.clone(),
            ItemId::Room { room_id } => format!("room:{room_id}"),
            ItemId::Vault { site } => format!("vault:{site}"),
            ItemId::File { name } => format!("file:{name}"),
            ItemId::PublicFile { name } => format!("public-file:{name}"),
            ItemId::System(item) => format!("system:{}", item.key()),
            ItemId::AllScreens => "screens:all".into(),
            ItemId::AddDevice => "action:add_device".into(),
            ItemId::JoinRoom => "action:join_room".into(),
        }
    }

    pub(crate) fn parse(text: &str) -> Result<ItemId, LayoutError> {
        if text.starts_with("direct~") || text.starts_with("room~") {
            return Ok(ItemId::Device { key: text.into() });
        }
        let invalid = || LayoutError::Item(text.to_string());
        let (prefix, rest) = text.split_once(':').ok_or_else(invalid)?;
        let item = match (prefix, rest) {
            (_, "") => return Err(invalid()),
            ("room", room_id) => ItemId::Room {
                room_id: room_id.into(),
            },
            ("vault", site) => ItemId::Vault { site: site.into() },
            ("file", name) => ItemId::File { name: name.into() },
            ("public-file", name) => ItemId::PublicFile { name: name.into() },
            ("system", key) => ItemId::System(SystemItem::parse_key(key).ok_or_else(invalid)?),
            ("screens", "all") => ItemId::AllScreens,
            ("action", "add_device") => ItemId::AddDevice,
            ("action", "join_room") => ItemId::JoinRoom,
            _ => return Err(invalid()),
        };
        Ok(item)
    }
}

impl TryFrom<String> for ItemId {
    type Error = LayoutError;

    fn try_from(text: String) -> Result<ItemId, LayoutError> {
        ItemId::parse(&text)
    }
}

impl From<ItemId> for String {
    fn from(item: ItemId) -> String {
        item.encode()
    }
}

/// A grid cell; columns grow to the right without bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Cell {
    pub(crate) col: u32,
    pub(crate) row: u32,
}

/// Arrangement of one monitor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LayoutFile", into = "LayoutFile")]
pub(crate) struct MonitorLayout {
    pub(crate) monitor: String,
    /// Rows of the work area; new items fill a column top to bottom.
    pub(crate) rows: u32,
    pub(crate) cells: BTreeMap<ItemId, Cell>,
}

/// File form: `{"schema":1,"monitor","rows","items":[{"id","col","row"}]}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LayoutFile {
    schema: u32,
    monitor: String,
    rows: u32,
    items: Vec<LayoutItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LayoutItem {
    id: ItemId,
    col: u32,
    row: u32,
}

impl TryFrom<LayoutFile> for MonitorLayout {
    type Error = LayoutError;

    fn try_from(file: LayoutFile) -> Result<MonitorLayout, LayoutError> {
        if file.schema != LAYOUT_SCHEMA {
            return Err(LayoutError::Schema(file.schema));
        }
        let mut layout = MonitorLayout::new(&file.monitor, file.rows)?;
        for item in file.items {
            let cell = Cell {
                col: item.col,
                row: item.row,
            };
            if layout.cells.contains_key(&item.id) || layout.item_at(cell).is_some() {
                return Err(LayoutError::Duplicate(item.id.encode()));
            }
            layout.cells.insert(item.id, cell);
        }
        Ok(layout)
    }
}

impl From<MonitorLayout> for LayoutFile {
    fn from(layout: MonitorLayout) -> LayoutFile {
        let mut items: Vec<LayoutItem> = layout
            .cells
            .into_iter()
            .map(|(id, cell)| LayoutItem {
                id,
                col: cell.col,
                row: cell.row,
            })
            .collect();
        items.sort_by_key(|item| (item.col, item.row));
        LayoutFile {
            schema: LAYOUT_SCHEMA,
            monitor: layout.monitor,
            rows: layout.rows,
            items,
        }
    }
}

impl MonitorLayout {
    pub(crate) fn new(monitor: &str, rows: u32) -> Result<MonitorLayout, LayoutError> {
        check_monitor_key(monitor)?;
        if rows == 0 {
            return Err(LayoutError::Rows);
        }
        Ok(MonitorLayout {
            monitor: monitor.to_string(),
            rows,
            cells: BTreeMap::new(),
        })
    }

    /// File name below `<AD>/desktop/layout/`.
    pub(crate) fn file_name(&self) -> String {
        format!("{}.json", self.monitor)
    }

    pub(crate) fn from_json(text: &str) -> Result<MonitorLayout, LayoutError> {
        serde_json::from_str(text).map_err(|error| LayoutError::Json(error.to_string()))
    }

    pub(crate) fn to_json(&self) -> Result<String, LayoutError> {
        serde_json::to_string_pretty(self).map_err(|error| LayoutError::Json(error.to_string()))
    }

    pub(crate) fn item_at(&self, cell: Cell) -> Option<&ItemId> {
        self.cells
            .iter()
            .find(|(_, placed)| **placed == cell)
            .map(|(item, _)| item)
    }

    /// First free cell, column by column from the top left.
    pub(crate) fn first_free(&self) -> Cell {
        let taken: std::collections::BTreeSet<Cell> = self.cells.values().copied().collect();
        let mut index: u64 = 0;
        loop {
            let cell = Cell {
                col: u32::try_from(index / u64::from(self.rows)).unwrap_or(u32::MAX),
                row: u32::try_from(index % u64::from(self.rows)).unwrap_or(0),
            };
            if !taken.contains(&cell) {
                return cell;
            }
            index += 1;
        }
    }

    fn place(&mut self, item: ItemId) -> Cell {
        let cell = self.first_free();
        self.cells.insert(item, cell);
        cell
    }

    /// Items in reading order of the columns.
    fn ordered_items(&self) -> Vec<ItemId> {
        let mut placed: Vec<(Cell, ItemId)> = self
            .cells
            .iter()
            .map(|(item, cell)| (*cell, item.clone()))
            .collect();
        placed.sort_by_key(|(cell, _)| (cell.col, cell.row));
        placed.into_iter().map(|(_, item)| item).collect()
    }
}

/// Changes the surface sends to the shell (`layout {op}`, C5.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LayoutOp {
    /// Dragged to a cell; an item already there moves to the first free cell.
    Move {
        item: ItemId,
        monitor: String,
        col: u32,
        row: u32,
    },
    /// Context menu "Auf Bildschirm verschieben" (Ä27): first free cell there.
    MoveToMonitor { item: ItemId, monitor: String },
    /// "Anordnen": the surface's order of its items, column by column.
    Arrange { monitor: String, order: Vec<ItemId> },
    /// The work area holds `rows` rows; items below move to free cells.
    Grid { monitor: String, rows: u32 },
}

/// Arrangement of every monitor the shell knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DesktopLayout {
    pub(crate) monitors: BTreeMap<String, MonitorLayout>,
}

impl DesktopLayout {
    /// The arrangement of a monitor; a new monitor starts empty.
    pub(crate) fn monitor_mut(&mut self, key: &str) -> Result<&mut MonitorLayout, LayoutError> {
        match self.monitors.entry(key.to_string()) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => Ok(entry.insert(MonitorLayout::new(key, DEFAULT_ROWS)?)),
        }
    }

    /// Monitor and cell of an item.
    pub(crate) fn locate(&self, item: &ItemId) -> Option<(&str, Cell)> {
        self.monitors
            .iter()
            .find_map(|(key, layout)| layout.cells.get(item).map(|cell| (key.as_str(), *cell)))
    }

    /// New items (on no monitor yet) go to the primary monitor in the given
    /// order (C4.2); placed items stay where they are. Returns the new ones.
    pub(crate) fn place_new_items(
        &mut self,
        primary: &str,
        items: &[ItemId],
    ) -> Result<Vec<ItemId>, LayoutError> {
        let new: Vec<ItemId> = items
            .iter()
            .filter(|item| self.locate(item).is_none())
            .cloned()
            .collect();
        let layout = self.monitor_mut(primary)?;
        for item in &new {
            if !layout.cells.contains_key(item) {
                layout.place(item.clone());
            }
        }
        Ok(new)
    }

    /// Monitors missing from `present` hand their items to the primary
    /// monitor for good, in their reading order (C5.5). Returns their keys:
    /// the os layer deletes `layout/<key>.json` of each.
    pub(crate) fn adopt_removed_monitors(
        &mut self,
        primary: &str,
        present: &[String],
    ) -> Result<Vec<String>, LayoutError> {
        let removed: Vec<String> = self
            .monitors
            .keys()
            .filter(|key| key.as_str() != primary && !present.contains(key))
            .cloned()
            .collect();
        let mut adopted = Vec::new();
        for key in &removed {
            if let Some(layout) = self.monitors.remove(key) {
                adopted.extend(layout.ordered_items());
            }
        }
        let target = self.monitor_mut(primary)?;
        for item in adopted {
            target.place(item);
        }
        Ok(removed)
    }

    /// Desktop files that left the folder leave the arrangement; devices,
    /// rooms and vaults keep their cell while they are briefly missing.
    pub(crate) fn forget_missing_files(&mut self, files: &[ItemId]) {
        for layout in self.monitors.values_mut() {
            layout.cells.retain(|item, _| {
                !matches!(item, ItemId::File { .. } | ItemId::PublicFile { .. })
                    || files.contains(item)
            });
        }
    }

    pub(crate) fn apply(&mut self, op: &LayoutOp) -> Result<(), LayoutError> {
        match op {
            LayoutOp::Move {
                item,
                monitor,
                col,
                row,
            } => {
                let cell = Cell {
                    col: *col,
                    row: *row,
                };
                // Check the target before the item leaves its cell.
                self.monitor_mut(monitor)?;
                self.remove(item);
                let layout = self.monitor_mut(monitor)?;
                let occupant = layout.item_at(cell).cloned();
                if let Some(occupant) = &occupant {
                    layout.cells.remove(occupant);
                }
                layout.cells.insert(item.clone(), cell);
                if let Some(occupant) = occupant {
                    layout.place(occupant);
                }
            }
            LayoutOp::MoveToMonitor { item, monitor } => {
                self.monitor_mut(monitor)?;
                self.remove(item);
                self.monitor_mut(monitor)?.place(item.clone());
            }
            LayoutOp::Arrange { monitor, order } => {
                let layout = self.monitor_mut(monitor)?;
                let mut items: Vec<ItemId> = order
                    .iter()
                    .filter(|item| layout.cells.contains_key(*item))
                    .cloned()
                    .collect();
                for item in layout.ordered_items() {
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
                layout.cells.clear();
                for item in items {
                    layout.place(item);
                }
            }
            LayoutOp::Grid { monitor, rows } => {
                if *rows == 0 {
                    return Err(LayoutError::Rows);
                }
                let layout = self.monitor_mut(monitor)?;
                layout.rows = *rows;
                let below: Vec<ItemId> = layout
                    .ordered_items()
                    .into_iter()
                    .filter(|item| layout.cells.get(item).is_some_and(|cell| cell.row >= *rows))
                    .collect();
                for item in below {
                    layout.cells.remove(&item);
                    layout.place(item);
                }
            }
        }
        Ok(())
    }

    fn remove(&mut self, item: &ItemId) {
        for layout in self.monitors.values_mut() {
            layout.cells.remove(item);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayoutError {
    Item(String),
    Monitor(String),
    Rows,
    Schema(u32),
    Duplicate(String),
    Json(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::Item(text) => write!(formatter, "unbekanntes Element {text:?}"),
            LayoutError::Monitor(key) => write!(formatter, "ungültiger Monitorschlüssel {key:?}"),
            LayoutError::Rows => write!(formatter, "ein Raster braucht mindestens eine Zeile"),
            LayoutError::Schema(schema) => {
                write!(formatter, "Anordnung mit unbekanntem Schema {schema}")
            }
            LayoutError::Duplicate(item) => {
                write!(formatter, "Element oder Zelle doppelt belegt: {item}")
            }
            LayoutError::Json(error) => write!(formatter, "Anordnung ungültig: {error}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Monitor keys are file name stems: 1 to 64 of `A-Z a-z 0-9 . _ -`, not
/// starting with a dot.
fn check_monitor_key(key: &str) -> Result<(), LayoutError> {
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
    if key.is_empty()
        || key.len() > MAX_MONITOR_KEY
        || key.starts_with('.')
        || !key.bytes().all(allowed)
    {
        return Err(LayoutError::Monitor(key.to_string()));
    }
    Ok(())
}
