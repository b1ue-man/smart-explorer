//! Explicit offline administrative provisioning. No service, firewall or active grant is changed.
use std::{io, path::PathBuf};
use zeroize::Zeroizing;
use crate::login::{core::{gateway_types::{GatewayNetwork, GatewayTarget}, machine::MachineIdentity,
    model::{PeerGrant, Policy, VERSION}}, os::platform};
use super::denied;
pub(crate) struct ProvisionRequest {
    /// A new private administrative directory, outside active service state.
    pub directory: PathBuf,
    /// Complete explicit administrative peer/user/lifetime/right choices.
    pub grants: Vec<PeerGrant>,
    pub network: GatewayNetwork,
}
#[derive(serde::Serialize)]
pub(crate) struct Provisioned {
    pub identity_file: PathBuf,
    pub policy_file: PathBuf,
    pub network_file: PathBuf,
    pub target_file: PathBuf,
    pub target: GatewayTarget,
}
pub(crate) fn provision(request: ProvisionRequest) -> io::Result<Provisioned> {
    request.network.validate()?;
    if request.grants.is_empty() { return Err(denied("provisioning requires at least one explicit peer/user login grant")); }
    let active = platform::paths()?; let state = platform::gateway_state_directory()?;
    if [&active.config, &active.releases, &state].iter().any(|root|
        request.directory.starts_with(root) || root.starts_with(&request.directory)) {
        return Err(denied("offline provisioning must not overwrite or contain active service installation/state"));
    }
    let identity = MachineIdentity::generate()?;
    let policy = Policy { format: VERSION, revision: 1, machine_id: identity.machine_id.clone(), machine_public_key: identity.public_key.clone(),
        gateway_identity: "SET_BY_ADMINISTRATIVE_INSTALLER".into(), grants: request.grants };
    policy.validate().map_err(denied)?;
    if policy.grants.iter().any(|grant| !grant.rights.login || !grant.rights.screen_view) {
        return Err(denied("every provisioned grant needs explicit login and screen-view rights"));
    }
    let private = Zeroizing::new(serde_json::to_vec_pretty(&identity).map_err(io::Error::other)?);
    let policy_bytes = serde_json::to_vec_pretty(&policy).map_err(io::Error::other)?;
    let network = serde_json::to_vec_pretty(&request.network).map_err(io::Error::other)?;
    if private.len() > 8192 || policy_bytes.len() > crate::login::core::model::MAX_POLICY
        || network.len() > crate::login::core::gateway_types::MAX_NETWORK { return Err(denied("provisioned identity, policy or network exceeds its installed record limit")); }
    let target = GatewayTarget { format: VERSION, machine_id: identity.machine_id.clone(), public_key: identity.public_key.clone(),
        direct_addresses: Vec::new(), relay_urls: request.network.relay_urls.clone(), use_default_relays: request.network.use_default_relays,
        allow_plaintext_relays: request.network.allow_plaintext_relays };
    target.validate()?;
    let public = serde_json::to_vec_pretty(&target).map_err(io::Error::other)?;
    if public.len() > crate::login::core::gateway_types::MAX_NETWORK { return Err(denied("provisioned public target exceeds its record limit")); }
    platform::write_provision_bundle(&request.directory, &[("machine-key.json", private.as_slice()), ("policy.json", &policy_bytes),
        ("gateway-network.json", &network), ("gateway-target.json", &public)])?;
    Ok(Provisioned { identity_file: request.directory.join("machine-key.json"), policy_file: request.directory.join("policy.json"),
        network_file: request.directory.join("gateway-network.json"), target_file: request.directory.join("gateway-target.json"), target })
}
