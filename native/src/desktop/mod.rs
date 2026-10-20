//! se Desktop for Windows and Linux (S5, plan
//! `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`).
//!
//! DSK-M1 delivers the platform-neutral core: module state with registration
//! ledger, mechanism and background decisions, tile projection, per-monitor
//! arrangement, configuration, the shell/surface channel and the S4 viewer
//! target. DSK-M3 adds the frame under `os/`: enable, disable and remove
//! with the ledger, the shell (`--desktop-shell`) with its surfaces
//! (`--desktop-surface`), the read without service start, the status and
//! the start switches. Inventory: `docs/oberflaechen/desktop-launcher.md`.
// The start switches (`run_gui`), `se desktop`, MCP `status` and Windows
// `--unregister` reach this module. The blanket allow stays, because each
// platform leaves the code of the other one unused (DSK-M6 removed the
// staging of the Linux names `FitMode::from_gnome`, `SystemBackground` and
// `SurfaceSignals::raise`; their allows in `desktop_surface_host.rs` are
// gone). Unused on Linux: `FitMode::from_windows`, the Windows parts and
// registrations, `ShellOutcome` (the Explorer actions). Unused on Windows:
// `FitMode::from_gnome`, the X11, layer and GNOME values of `Mechanism` and
// `Reason`, `DisplayServer::{X11, Wayland}`, the Linux ledger kinds.
#![allow(dead_code)]

pub(crate) mod core;
pub(crate) mod os;

pub(crate) use os::{remove_system_traces, resume_on_gui_start, run_if_requested, status_value};
