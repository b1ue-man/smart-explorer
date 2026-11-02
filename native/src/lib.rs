//! Smart Explorer: the programs (`smart_explorer`, `se`, the updater and the MCP
//! launcher) over the workspace crates (plan docs/plaene/2026-11-01-idealzustand,
//! D13): `smart_explorer_engine` (everything the surfaces share),
//! `smart_explorer_gui` (explorer window, se Desktop), `smart_explorer_cli` (`se`)
//! and `smart_explorer_mobile` (Android facade). This library re-exports them,
//! so `smart_explorer::<module>` keeps naming every module for the programs, the
//! integration tests and the Android bridge.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

#[cfg(not(target_os = "android"))]
pub use smart_explorer_cli::*;
#[cfg(target_os = "android")]
pub use smart_explorer_mobile::*;

/// The explorer window with every panel of the crates above the GUI: the CLI
/// registers its panel of saved terminal/MCP commands first.
#[cfg(not(target_os = "android"))]
pub fn run_gui() -> eframe::Result<()> {
    smart_explorer_cli::cli::register_gui_panels();
    smart_explorer_gui::run_gui()
}
