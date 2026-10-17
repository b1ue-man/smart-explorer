//! Entries of the surface besides the Share tiles (C1.2, C2.7, C4.2): desktop
//! files, the Windows items "Papierkorb" and "Dieser PC", and the order in
//! which new items are placed (C1.2: by name, column by column from the top
//! left; Explorer's own icon positions are not read, Annahme A11).
use serde::{Deserialize, Serialize};

use super::config::DesktopConfig;
use super::layout::ItemId;
use super::state::Platform;
use super::tiles::TileModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemItem {
    RecycleBin,
    ThisPc,
}

impl SystemItem {
    pub(crate) const ALL: [SystemItem; 2] = [SystemItem::RecycleBin, SystemItem::ThisPc];

    /// Key in `config.json` (`system_items`) and in layout IDs.
    pub(crate) fn key(self) -> &'static str {
        match self {
            SystemItem::RecycleBin => "recycle_bin",
            SystemItem::ThisPc => "this_pc",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            SystemItem::RecycleBin => "Papierkorb",
            SystemItem::ThisPc => "Dieser PC",
        }
    }

    /// Value name under `HideDesktopIcons\NewStartPanel`, read once when the
    /// module is enabled (Ä5).
    pub(crate) fn hide_icons_value(self) -> &'static str {
        match self {
            SystemItem::RecycleBin => "{645FF040-5081-101B-9F08-00AA002F954E}",
            SystemItem::ThisPc => "{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        }
    }

    /// Opened with `explorer.exe <shell folder>`.
    pub(crate) fn shell_folder(self) -> &'static str {
        match self {
            SystemItem::RecycleBin => "shell:RecycleBinFolder",
            SystemItem::ThisPc => "shell:MyComputerFolder",
        }
    }

    pub(crate) fn parse_key(key: &str) -> Option<SystemItem> {
        SystemItem::ALL.into_iter().find(|item| item.key() == key)
    }

    fn enabled(self, config: &DesktopConfig) -> bool {
        match self {
            SystemItem::RecycleBin => config.system_items.recycle_bin,
            SystemItem::ThisPc => config.system_items.this_pc,
        }
    }
}

/// One entry of the desktop folder (Windows `FOLDERID_Desktop` and
/// `FOLDERID_PublicDesktop`, Linux `XDG_DESKTOP_DIR`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DesktopFile {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    /// From the all-users desktop (Windows); renaming needs administrator rights.
    pub(crate) public: bool,
    /// Linux starter `.desktop`, started only under the trust rule of C2.7.
    pub(crate) launcher: bool,
    /// Hidden (Windows attribute, Linux leading dot); never shown.
    pub(crate) hidden: bool,
}

impl DesktopFile {
    pub(crate) fn item_id(&self) -> ItemId {
        if self.public {
            ItemId::PublicFile {
                name: self.name.clone(),
            }
        } else {
            ItemId::File {
                name: self.name.clone(),
            }
        }
    }
}

/// Visible files in placement order: by name without case, then by the
/// exact name, the user's file before an all-users file of the same name.
pub(crate) fn visible_files(files: &[DesktopFile]) -> Vec<DesktopFile> {
    let mut visible: Vec<DesktopFile> = files.iter().filter(|file| !file.hidden).cloned().collect();
    visible.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.public.cmp(&right.public))
    });
    visible
}

/// Every item of the surface in placement order: system items (Windows),
/// desktop files, devices, rooms, vaults, "Alle Bildschirme", then the
/// empty-state tiles. Hidden groups are left out (C4.2); room members are
/// placed when their room is opened (DSK-M4), not here.
pub(crate) fn surface_items(
    platform: Platform,
    config: &DesktopConfig,
    files: &[DesktopFile],
    tiles: &TileModel,
) -> Vec<ItemId> {
    let groups = config.groups;
    let mut items = Vec::new();
    if platform == Platform::Windows {
        let shown = SystemItem::ALL
            .into_iter()
            .filter(|item| item.enabled(config));
        items.extend(shown.map(ItemId::System));
    }
    if groups.desktop_files {
        items.extend(visible_files(files).iter().map(DesktopFile::item_id));
    }
    if groups.devices {
        items.extend(tiles.devices.iter().map(|tile| ItemId::Device {
            key: tile.key.clone(),
        }));
    }
    if groups.rooms {
        items.extend(tiles.rooms.iter().map(|room| ItemId::Room {
            room_id: room.room_id.clone(),
        }));
    }
    if let Some(vaults) = tiles.vaults.as_ref().filter(|_| groups.vaults) {
        items.extend(vaults.iter().map(|vault| ItemId::Vault {
            site: vault.site.clone(),
        }));
    }
    if groups.screens {
        items.push(ItemId::AllScreens);
    }
    if groups.devices || groups.rooms {
        items.extend(tiles.empty_actions.iter().map(|action| action.item_id()));
    }
    items
}
