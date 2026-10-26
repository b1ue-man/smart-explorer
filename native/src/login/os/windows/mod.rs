//! Windows service identity, no network listener, fresh console helper per desktop.
mod protected;
mod security;
mod pipe;
mod service;
mod host_status;
pub(crate) use host_status::{host_process_instance, host_uptime_ms, read_host_evidence, write_host_evidence};
mod session;
mod spawn;
mod sas;
pub(crate) use protected::{open_protected, paths, gateway_state_directory, ProtectedFile};
pub(crate) use pipe::{connect, helper_channel, ClientStream};
pub(crate) use service::run_broker;
pub(crate) use session::{binding, finish_helper, helper_binding_matches, prepare_helper, run_probe};
pub(crate) use spawn::spawn;
pub(crate) use sas::secure_attention;
use std::io;
use crate::login::os::shared::denied;
pub(crate) const SERVICE: &str = "SmartExplorerLoginBroker";
pub(crate) const GATEWAY: &str = "SmartExplorerLoginGateway";
pub(crate) const PIPE: &str = r"\\.\pipe\SmartExplorerLoginBroker";
pub(crate) fn validate_gateway_identity(identity: &str) -> io::Result<()> {
    // The resolved SCM service SID is an administrative policy value, never a user SID.
    if !identity.starts_with("S-1-5-80-") || identity != security::service_sid(GATEWAY)? {
        return Err(denied("login gateway identity must be the exact dedicated gateway service SID"));
    }
    Ok(())
}

pub(crate) use protected::{read_gateway_key, read_gateway_network, write_gateway_address, write_provision_bundle};
pub(crate) use pipe::require_gateway;
pub(crate) use service::run_gateway_service;
