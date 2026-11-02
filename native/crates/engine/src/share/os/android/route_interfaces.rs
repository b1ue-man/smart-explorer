//! Android uses the same kernel route/hardware proof. SELinux denial stays unknown.
//! Interface names and private/ULA/CGNAT ranges never manufacture a LAN verdict.
#[path = "../linux_os/route_interfaces.rs"]
mod kernel;

pub(super) fn physical_on_link(
    local: Option<std::net::IpAddr>,
    remote: std::net::SocketAddr,
) -> bool {
    kernel::physical_on_link(local, remote)
}
