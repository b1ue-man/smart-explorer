//! "Papierkorb" and "Dieser PC" on the surface (DSK-M5, C1.2, Ä5): own se
//! settings (`system_items.recycle_bin`, `system_items.this_pc`) whose start
//! values come once, at activation, from Explorer's per-user switches under
//! `HKCU\…\HideDesktopIcons\NewStartPanel`: value 0 shown, any other value
//! hidden, no value the se default (Papierkorb on, Dieser PC off). Afterwards
//! se follows only its own setting and never writes Explorer's values.
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use super::nav_pane::HIDE_ICONS;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::items::SystemItem;

/// The start values from Explorer's switches of this user.
pub(crate) fn initial_system_items() -> SystemItems {
    initial_from(HIDE_ICONS)
}

/// The start values from the `REG_DWORD` values under `HKCU\<key>`; a
/// missing key, a missing value and a value of another type count as
/// missing. The task test reads a key of its own here.
pub(crate) fn initial_from(key: &str) -> SystemItems {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(key);
    let read = |item: SystemItem| {
        key.as_ref()
            .ok()
            .and_then(|key| key.get_value::<u32, _>(item.hide_icons_value()).ok())
    };
    SystemItems::initial(read(SystemItem::RecycleBin), read(SystemItem::ThisPc))
}
