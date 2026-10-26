//! Desktop-app files of strand S2 (drives) of plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“. `app/mod.rs` includes this file once (R2); later S2
//! packages declare their app files here instead of in `app/mod.rs`.

// DRV-M3: „Einhängeort“ and „Beim Anmelden automatisch einbinden“ of the
// window „Remote als Laufwerk“.
#[path = "core/mount_ui_location.rs"]
pub(crate) mod mount_ui_location;

/// Platform fact for `mount_ui_location`: Windows drives sit at a drive
/// letter (strand S2 C1); every other desktop platform places a drive at a
/// folder (S2 C2, DRV-M4). Kept out of `core/` (AGENTS.md).
pub(crate) const DRIVES_USE_LETTERS: bool = cfg!(windows);

// DRV-M5: „Im Dateimanager öffnen“ and the one-time hint of the device tree.
#[path = "core/mount_ui_open.rs"]
pub(crate) mod mount_ui_open;
// DRV-M6: Einstellungen → Integration → „Verbundene Geräte als Ordner“.
#[path = "core/settings_devices_ui.rs"]
pub(crate) mod settings_devices_ui;

#[path = "os/shared/drive_events.rs"]
pub(crate) mod drive_events;
#[path = "os/shared/drive_inventory.rs"]
pub(crate) mod drive_inventory;
#[path = "core/drive_snapshot.rs"]
pub(crate) mod drive_snapshot;
#[path = "os/shared/optical_index.rs"]
pub(crate) mod optical_index;
