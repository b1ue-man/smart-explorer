//! Windows service identity, no network listener, fresh console helper per desktop.
mod host_status;
mod pipe;
mod protected;
mod security;
mod self_platform;
mod service;
pub use host_status::{
    host_process_instance, host_uptime_ms, read_host_evidence, write_host_evidence,
};
mod sas;
mod session;
mod spawn;
use crate::login::os::shared::denied;
pub use pipe::{connect, helper_channel, ClientStream};
pub use protected::{gateway_state_directory, open_protected, paths, ProtectedFile};
pub use sas::secure_attention;
pub use self_platform::{
    connect_grants, create_stage_file, elevation, open_image, privileged, read_self_record,
    run_elevated, run_install_script, stage_root, write_self_record, APP_FILE, SCRIPT, SCRIPT_NAME,
    STAGE_IMAGE,
};
pub use service::run_broker;
pub use session::{binding, finish_helper, helper_binding_matches, prepare_helper, run_probe};
pub use spawn::spawn;
use std::io;
pub const SERVICE: &str = "SmartExplorerLoginBroker";
pub const GATEWAY: &str = "SmartExplorerLoginGateway";
pub const PIPE: &str = r"\\.\pipe\SmartExplorerLoginBroker";
pub fn validate_gateway_identity(identity: &str) -> io::Result<()> {
    // The resolved SCM service SID is an administrative policy value, never a user SID.
    if !identity.starts_with("S-1-5-80-") || identity != security::service_sid(GATEWAY)? {
        return Err(denied(
            "login gateway identity must be the exact dedicated gateway service SID",
        ));
    }
    Ok(())
}

pub use pipe::require_gateway;
pub use protected::{
    read_gateway_key, read_gateway_network, write_gateway_address, write_provision_bundle,
};
pub use service::run_gateway_service;
