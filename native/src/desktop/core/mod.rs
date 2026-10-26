//! Platform-neutral core of se Desktop (DSK-M1). No platform branches, no OS
//! crates, no FFI: the os layer passes every fact in as a typed value and
//! performs every effect (files, registry, windows, processes) itself.
pub(crate) mod config;
pub(crate) mod folder_open;
pub(crate) mod items;
pub(crate) mod layout;
pub(crate) mod ledger;
pub(crate) mod mechanism;
pub(crate) mod protocol;
pub(crate) mod screen_target;
pub(crate) mod state;
pub(crate) mod tiles;

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
