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
// Staged: R5 wires the start switches into `run_gui`, `se desktop` into the
// CLI and `status_value` into MCP `status`; until then only the task tests
// and the settings reach this module.
#![allow(dead_code)]

pub(crate) mod core;
pub(crate) mod os;

pub(crate) use os::{remove_system_traces, resume_on_gui_start, run_if_requested, status_value};
