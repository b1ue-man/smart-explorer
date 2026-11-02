//! Transfer layer of Smart Explorer (plan docs/plaene/2026-11-01-idealzustand,
//! D13): copying, the transfer engine, synchronisation and the SFTP, SMB and
//! agent backends. It re-exports the storage layer, so `crate::<module>`
//! paths reach the layers below.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

pub use smart_explorer_storage::*;

pub mod agent;
pub mod bisync;
pub mod copy;
pub mod sftp;
pub mod smb;
pub mod sync;
pub mod syncjobs;
pub mod transfer;
pub mod watch;
pub mod worker_log;
pub mod zipfs;

// The ports of the higher layers in this crate's own tests.
#[cfg(test)]
mod test_providers;
