//! Storage layer of Smart Explorer (plan docs/plaene/2026-11-01-idealzustand,
//! D13): the virtual file system, its local and FTP backends, the agent wire
//! protocol, scans and analyses. It re-exports the core layer, so
//! `crate::<module>` paths reach those modules.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]
// The helpers behind `test-support` serve the tests of the crates above
// (fixtures, hooks, counters); this crate's own build without its tests has
// no caller for some of them and for the imports of their `#[test]` items.
#![cfg_attr(
    all(feature = "test-support", not(test)),
    allow(dead_code, unused_imports)
)]

pub use smart_explorer_core::*;

pub mod agent_proto;
pub mod analytics;
#[cfg(unix)]
pub mod android_fs;
pub mod apptrash;
pub mod cloud;
pub mod filter;
pub mod ftp;
pub mod host_trash;
pub mod rscan;
pub mod scanner;
pub mod source_media;
pub mod vault_error;
pub mod vfs;

// Stand-ins of the storage ports for the tests of storage and transfer.
#[cfg(any(test, feature = "test-support"))]
pub mod test_ports;
#[cfg(test)]
mod test_providers;
