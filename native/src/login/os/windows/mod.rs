//! Windows service identity, no network listener, fresh console helper per desktop.
mod host_status;
mod pipe;
mod protected;
mod security;
mod service;
pub(crate) use host_status::{
    host_process_instance, host_uptime_ms, read_host_evidence, write_host_evidence,
};
mod sas;
mod session;
mod spawn;
use crate::login::os::shared::denied;
pub(crate) use pipe::{connect, helper_channel, ClientStream};
pub(crate) use protected::{gateway_state_directory, open_protected, paths, ProtectedFile};
pub(crate) use sas::secure_attention;
pub(crate) use service::run_broker;
pub(crate) use session::{
    binding, finish_helper, helper_binding_matches, prepare_helper, run_probe,
};
pub(crate) use spawn::spawn;
use std::io;
pub(crate) const SERVICE: &str = "SmartExplorerLoginBroker";
pub(crate) const GATEWAY: &str = "SmartExplorerLoginGateway";
pub(crate) const PIPE: &str = r"\\.\pipe\SmartExplorerLoginBroker";
pub(crate) fn validate_gateway_identity(identity: &str) -> io::Result<()> {
    // The resolved SCM service SID is an administrative policy value, never a user SID.
    if !identity.starts_with("S-1-5-80-") || identity != security::service_sid(GATEWAY)? {
        return Err(denied(
            "login gateway identity must be the exact dedicated gateway service SID",
        ));
    }
    Ok(())
}

pub(crate) use pipe::require_gateway;
pub(crate) use protected::{
    read_gateway_key, read_gateway_network, write_gateway_address, write_provision_bundle,
};
pub(crate) use service::run_gateway_service;
