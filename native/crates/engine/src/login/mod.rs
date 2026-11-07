//! Optional administrative login service, V1 / D7 P1b–P1c.
//!
//! Integration: register this module in lib.rs; dispatch BROKER_SWITCH through
//! run_broker and HELPER_SWITCH/PROBE_SWITCH through run_helper before GUI startup.
//! The boot gateway runs separately under its service identity, owns its own
//! Iroh machine key, forwards peer-signed operations through GatewayClient and
//! encodes returned raw pixels without loading a codec into the broker/helper.
//! Existing user daemons and account-exec services retain their entrypoints.
//! Install scripts are explicit administrative actions; a per-user update never
//! installs, replaces, or enables this service.
pub mod core;
pub mod os;
pub use core::model::{
    Challenge, DesktopKind, LoginRights, Operation, PeerGrant, Policy, SessionBinding,
    SignedRequest, BROKER_SWITCH, GATEWAY_SWITCH, HELPER_SWITCH, PROBE_SWITCH,
};
pub use core::wire::Reply;
pub use os::shared::client::GatewayClient;

pub fn run_broker() -> std::io::Result<()> {
    os::platform::run_broker()
}
pub fn run_helper(probe: bool) -> std::io::Result<()> {
    os::shared::helper::run(probe)
}
pub fn policy() -> std::io::Result<Policy> {
    os::shared::installation::policy()
}

pub use core::machine::MachineIdentity;
pub fn gateway_state_directory() -> std::io::Result<std::path::PathBuf> {
    os::platform::gateway_state_directory()
}

// Main dispatches the already isolated service switch; this never elevates a user daemon.
pub fn run_gateway() -> std::io::Result<()> {
    os::platform::run_gateway_service()
}
pub use crate::share::login_transport::RouteCancellation as LoginCancellation;
pub use core::gateway_types::{GatewayAccess, GatewayNetwork, GatewayReply, GatewayTarget};
pub use core::viewer_guard::FrameToken;
pub use os::shared::gateway_client::RemoteLoginClient;
pub use os::shared::host_status::activity as host_activity;
pub use os::shared::provision::{
    provision, provision_network, ProvisionRequest, Provisioned, ProvisionedNetwork,
};
pub use os::shared::status::{installation_status, InstallationStatus};
pub use os::shared::target::read_target_file;
pub use os::shared::viewer::{ViewerFrame, ViewerSession, ViewerSnapshot};
