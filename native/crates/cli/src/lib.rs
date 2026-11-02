//! The `se` command line of Smart Explorer (plan
//! docs/plaene/2026-11-01-idealzustand, D13). It builds on the GUI crate (se
//! Desktop control, start requests) and the engine, whose modules both
//! re-export, so `crate::<module>` paths reach them.

pub use smart_explorer_gui::*;

pub mod cli;
