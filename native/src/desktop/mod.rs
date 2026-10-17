//! se Desktop for Windows and Linux (S5, plan
//! `docs/plaene/2026-10-17-rechte-desktop-tresore/strang-desktop.md`).
//!
//! DSK-M1 delivers the platform-neutral core: module state with registration
//! ledger, mechanism and background decisions, tile projection, per-monitor
//! arrangement, configuration, the shell/surface channel and the S4 viewer
//! target. The shell (`--desktop-shell`), the surfaces (`--desktop-surface`)
//! and the platform adapters follow from DSK-M3 on under `os/`.
// Staged: DSK-M3 (U15) wires the core into the shell and the surfaces; until
// then only the task tests use it.
#![allow(dead_code)]

pub(crate) mod core;
