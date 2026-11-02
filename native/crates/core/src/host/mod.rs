//! The host the programs run on: its OS adapter for the transfer engine
//! (file replacement, canonical folders, positioned writes, thread priority,
//! devices and volumes), the memory budget, the load sensors and the
//! compression policy built on them, and the replacement of a local file. They need nothing above the core layer
//! (plan docs/plaene/2026-11-01-idealzustand, D13); `transfer` re-exports them
//! under its old paths, `agent_proto` and `analytics` name them here.

#[path = "core/compress_policy.rs"]
pub mod compress_policy;
#[cfg(windows)]
#[path = "os/windows_replace.rs"]
mod local_replace;
#[cfg(not(windows))]
#[path = "os/unix_replace.rs"]
mod local_replace;
#[path = "os/shared/memory.rs"]
mod memory;
// Restart-stable file identity, shared verbatim with se-agent (through agent_proto).
#[cfg(not(windows))]
#[path = "os/linux_file_proof.rs"]
mod linux_file_proof;
#[cfg(windows)]
#[path = "os/windows.rs"]
pub mod platform;
#[cfg(not(windows))]
#[path = "os/unix.rs"]
pub mod platform;
#[path = "core/sensor_parse.rs"]
pub mod sensor_parse;
#[path = "os/shared/sensors.rs"]
pub mod sensors;
#[cfg(windows)]
#[path = "os/windows_file_proof.rs"]
mod windows_file_proof;

#[cfg(test)]
#[path = "core/compress_policy_tests.rs"]
mod compress_policy_tests;
#[cfg(test)]
#[path = "core/sensor_parse_tests.rs"]
mod sensor_parse_tests;

#[cfg(not(windows))]
pub use linux_file_proof::stable_handle_identity;
pub use local_replace::replace_file as replace_local_file;
#[cfg(windows)]
pub use local_replace::wide as wide_local_path;
pub use memory::{memory_budget, reserve_memory, try_reserve_memory, MemoryReservation};
pub use platform::{available_memory, physical_memory};
#[cfg(windows)]
pub use windows_file_proof::{same_file_object, stable_handle_identity};
