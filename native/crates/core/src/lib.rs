//! Core layer of Smart Explorer (plan docs/plaene/2026-11-01-idealzustand,
//! D13): types, formats, the host adapter, private storage and local access
//! that every other crate builds on.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

pub mod creds;
pub mod cryptomator;
pub mod format;
pub mod host;
pub mod licenses;
pub mod linemerge;
pub mod local_access;
/// Ports: lower layers reach their higher implementations through the
/// linker (D13); `declare_port!` and `provide_port!` are defined there.
pub mod ports;
pub mod support_dirs;
pub mod types;
