//! Platform-neutral core of se Desktop (DSK-M1). No platform branches, no OS
//! crates, no FFI: the os layer passes every fact in as a typed value and
//! performs every effect (files, registry, windows, processes) itself.
pub mod config;
pub mod folder_open;
pub mod items;
pub mod layout;
pub mod ledger;
pub mod mechanism;
pub mod protocol;
pub mod screen_target;
pub mod state;
pub mod tiles;

#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod mechanism_tests;
#[cfg(test)]
mod protocol_tests;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tiles_tests;
