//! The host the programs run on: its OS adapter for the transfer engine
//! (file replacement, canonical folders, positioned writes, thread priority,
//! devices and volumes), the memory budget, the load sensors and the
//! compression policy built on them. They need nothing above the core layer
//! (plan docs/plaene/2026-11-01-idealzustand, D13); `transfer` re-exports them
//! under its old paths, `agent_proto` and `analytics` name them here.

#[path = "core/compress_policy.rs"]
pub mod compress_policy;
#[path = "os/shared/memory.rs"]
mod memory;
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

#[cfg(test)]
#[path = "core/compress_policy_tests.rs"]
mod compress_policy_tests;
#[cfg(test)]
#[path = "core/sensor_parse_tests.rs"]
mod sensor_parse_tests;

pub use memory::{memory_budget, reserve_memory, try_reserve_memory, MemoryReservation};
pub use platform::{available_memory, physical_memory};
