#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows/interfaces.rs"]
mod interfaces;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/interfaces.rs"]
mod interfaces;

#[path = "core/backend.rs"]
mod backend;
#[path = "core/link_facts.rs"]
mod link_facts;
#[path = "core/net.rs"]
mod imp;

pub use backend::UncBackend;
pub use imp::*;
pub use link_facts::{
    classify_links, is_link_local, parse_candidate, scoped_v6_candidate, InterfaceFacts,
    LinkClass,
};

/// Typed facts about every network interface, gathered by the OS adapter.
/// Never assumes a facility: an unreadable source is an `Err` with the reason.
pub fn gather_interface_facts() -> Result<Vec<InterfaceFacts>, String> {
    interfaces::gather_interface_facts()
}
