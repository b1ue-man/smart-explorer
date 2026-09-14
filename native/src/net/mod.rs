#[cfg(windows)]
#[path = "os/windows/interfaces.rs"]
mod interfaces;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/interfaces.rs"]
mod interfaces;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;

#[cfg(windows)]
#[path = "os/windows/ics.rs"]
mod ics;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/nm_shared.rs"]
mod nm_shared;
#[cfg(windows)]
#[path = "os/windows/uplink_adapter.rs"]
mod uplink_adapter;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/uplink_adapter.rs"]
mod uplink_adapter;
#[cfg(windows)]
#[path = "os/windows/uplink_helper.rs"]
mod uplink_helper;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/uplink_polkit.rs"]
mod uplink_polkit;
#[path = "os/shared/uplink_state_store.rs"]
mod uplink_state_store;

#[path = "core/backend.rs"]
mod backend;
#[path = "core/net.rs"]
mod imp;
#[path = "core/link_facts.rs"]
mod link_facts;
#[path = "core/uplink.rs"]
mod uplink;

pub use backend::UncBackend;
pub use imp::*;
pub use link_facts::{
    classify_links, is_link_local, parse_candidate, scoped_v6_candidate, InterfaceFacts, LinkClass,
};
pub use uplink::{
    valid_adapter_id, Facility, SharingRecord, UnsupportedAdapter, UplinkAdapter, UplinkState,
    UplinkTarget,
};

/// The platform's uplink-sharing implementation. Never assumes support: the
/// adapter's `probe` reports what is actually available on this host.
pub fn uplink_adapter() -> Box<dyn UplinkAdapter> {
    #[cfg(windows)]
    {
        Box::new(uplink_adapter::WindowsIcsAdapter::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(uplink_adapter::NetworkManagerAdapter::new())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(UnsupportedAdapter(
            "Internet-Teilen ist auf diesem Betriebssystem nicht implementiert".into(),
        ))
    }
}

/// Runs the elevated Windows helper when invoked as `se --lan-uplink-helper`.
/// `None` on every other invocation and on other platforms.
pub fn run_uplink_helper_if_requested(
    arguments: &[std::ffi::OsString],
) -> Option<std::io::Result<()>> {
    #[cfg(windows)]
    {
        uplink_adapter::run_helper_if_requested(arguments)
    }
    #[cfg(not(windows))]
    {
        let _ = arguments;
        None
    }
}

/// Typed facts about every network interface, gathered by the OS adapter.
/// Never assumes a facility: an unreadable source is an `Err` with the reason.
pub fn gather_interface_facts() -> Result<Vec<InterfaceFacts>, String> {
    interfaces::gather_interface_facts()
}
