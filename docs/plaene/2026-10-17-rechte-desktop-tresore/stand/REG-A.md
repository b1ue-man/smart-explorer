# Stand REG-A (Registrierung U16 und U17)

Eingetragen in die Hub-Dateien, geprüft gegen den committeten Code (U16: b6beea02, U17: 0b11d3e6). Kein Build, kein Test, kein Commit.

## Eingetragene Zeilen

- `native/Cargo.toml`: `[dependencies]` `png = "0.18"` (Cargo.lock führt `png 0.18.1`, `snapshot.rs` nutzt `png::Encoder`). `[target.'cfg(windows)'.dependencies]`, Liste `windows`: `Graphics`, `Graphics_Capture`, `Graphics_DirectX`, `Graphics_DirectX_Direct3D11`, `Win32_Graphics_Dwm`, `Win32_System_StationsAndDesktops`, `Win32_System_WinRT`, `Win32_System_WinRT_Direct3D11`, `Win32_System_WinRT_Graphics_Capture`, `Win32_UI_HiDpi`. Alle zehn Namen stehen in `windows-0.58.0/Cargo.toml`.
- `native/src/daemon/mod.rs`: `mod screen_ipc_still` (`#[path = "os/shared/screen_ipc_still.rs"]`); `pub(crate) use screen_ipc_still::{screen_snapshot, screen_snapshot_within, screen_windows, screen_windows_within, StillImage};`; `pub use drive_modules::devices_ipc::{devices_disable, devices_enable, devices_hint_done, devices_hint_due, devices_status, devices_watch, DeviceStatusRow, DevicesStatus};`.
- `native/src/cli/mcp/catalog.rs`: `use super::screen_tool;`; Werkzeug `screen` am Ende von `tools()` (nur lesend, nicht upfront); `INSTRUCTIONS`: „(exec, job_input, job_cancel, sessions, status, screen, se)“; Beispiele `["drive","devices","status","--json"], ["drive","open","<id>"]` nach dem Linux-Beispiel von `drive mount` und `["screen","windows","<device>","--json"]` nach `["screen","codec","--json"]`; Test `mcp_task_catalog_is_complete_and_within_host_limits`: `"screen"` am Ende der Namensliste (nach `sessions`), in der Liste der nur lesenden und in der Liste der nicht vorgeladenen Werkzeuge.
- `native/src/daemon/os/shared/ipc_protocol_mount.rs`: Feld `device_tree: bool` (`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`) nach `location`; in `From<&MountConfig>` `device_tree: config.source.is_device_tree(),`.
- `native/src/daemon/os/shared/ipc_protocol.rs`: Variante `Devices { status: super::drive_modules::devices_ipc::DevicesStatus }` vor `Err`.
- `native/src/daemon/os/shared/ipc_host_ext.rs`: Arm `MountHostExt` ruft `super::super::drive_modules::mount_host_ext_serve::serve(self, stream, &id, req)`.
- `native/src/daemon/drive_modules.rs`: `devices_host` (privat), `devices_ipc`, `devices_mount`, `mount_host_ext_serve` (je `pub(crate)`), jeweils mit eigener `#[path]`-Zeile.

## Abweichungen

- Keine inhaltliche Abweichung bei `INSTRUCTIONS`; „screen“ steht unmittelbar vor „se“, wörtlich nach U16.
- Die vier Moduldeklarationen in `drive_modules.rs` stehen hinter `ipc_protocol_mount_host` statt hinter `ipc_protocol_drives`, in der Reihenfolge `devices_host`, `devices_ipc`, `devices_mount`, `mount_host_ext_serve` (rustfmt-Reihenfolge; Verhalten gleich).
- Alle Namen und Sichtbarkeiten stimmten mit den Quellen überein (`serve` ist `pub(in crate::daemon)`, vom Arm in `crate::daemon` erreichbar; `serve_host_ext` gibt es nicht mehr). Keine Namensabweichung.

## Dateigröße

Keine geänderte Datei erreicht 500 Zeilen (größte: `ipc_protocol.rs` mit 461, `catalog.rs` mit 363). Formatprüfung: alle sechs Rust-Dateien ohne Ausgabe.

## Offene Punkte

- `native/Cargo.lock` unberührt (Erneuerung aus dem Suite-Artefakt); `png 0.18.1` steht schon darin.
- Nicht übersetzt, kein Test gelaufen; die Windows-Features sind nur durch die Windows-Jobs prüfbar.
