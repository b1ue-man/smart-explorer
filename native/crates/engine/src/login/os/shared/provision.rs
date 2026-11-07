//! Explicit offline administrative provisioning. No service, firewall or active grant is changed.
use super::denied;
use crate::login::{
    core::{
        gateway_types::{GatewayNetwork, GatewayTarget},
        machine::MachineIdentity,
        model::{PeerGrant, Policy, VERSION},
    },
    os::platform,
};
use std::{
    io,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
pub struct ProvisionRequest {
    /// A new private administrative directory, outside active service state.
    pub directory: PathBuf,
    /// Complete explicit administrative peer/user/lifetime/right choices.
    pub grants: Vec<PeerGrant>,
    pub network: GatewayNetwork,
}
#[derive(serde::Serialize)]
pub struct Provisioned {
    pub identity_file: PathBuf,
    pub policy_file: PathBuf,
    pub network_file: PathBuf,
    pub target_file: PathBuf,
    pub target: GatewayTarget,
}
/// `use_default_relays: true` means the relays of the Share server
/// configured in this program (E14, never number0). The gateway service reads
/// no user profile, so provisioning resolves them here into explicit relays,
/// pins and plaintext rule; without a configured server only direct paths.
fn resolve_network(network: GatewayNetwork) -> io::Result<GatewayNetwork> {
    if !network.use_default_relays {
        return Ok(network);
    }
    let server = crate::daemon::load_share_server().map_err(io::Error::other)?;
    let resolved = crate::share::login_transport::configured_server_network(&server);
    resolved.validate()?;
    Ok(resolved)
}
/// Refuses a directory that is, contains, or lies inside the active service
/// installation or the gateway state. Shared by `provision` and `provision_network`.
fn check_location(directory: &Path) -> io::Result<()> {
    let active = platform::paths()?;
    let state = platform::gateway_state_directory()?;
    if [&active.config, &active.releases, &state]
        .iter()
        .any(|root| directory.starts_with(root) || root.starts_with(directory))
    {
        return Err(denied(
            "offline provisioning must not overwrite or contain active service installation/state",
        ));
    }
    Ok(())
}
pub fn provision(mut request: ProvisionRequest) -> io::Result<Provisioned> {
    request.network.validate()?;
    request.network = resolve_network(request.network)?;
    if request.grants.is_empty() {
        return Err(denied(
            "provisioning requires at least one explicit peer/user login grant",
        ));
    }
    check_location(&request.directory)?;
    let identity = MachineIdentity::generate()?;
    let policy = Policy {
        format: VERSION,
        revision: 1,
        machine_id: identity.machine_id.clone(),
        machine_public_key: identity.public_key.clone(),
        gateway_identity: "SET_BY_ADMINISTRATIVE_INSTALLER".into(),
        grants: request.grants,
    };
    policy.validate().map_err(denied)?;
    if policy
        .grants
        .iter()
        .any(|grant| !grant.rights.login || !grant.rights.screen_view)
    {
        return Err(denied(
            "every provisioned grant needs explicit login and screen-view rights",
        ));
    }
    let private = Zeroizing::new(serde_json::to_vec_pretty(&identity).map_err(io::Error::other)?);
    let policy_bytes = serde_json::to_vec_pretty(&policy).map_err(io::Error::other)?;
    let network = serde_json::to_vec_pretty(&request.network).map_err(io::Error::other)?;
    if private.len() > 8192
        || policy_bytes.len() > crate::login::core::model::MAX_POLICY
        || network.len() > crate::login::core::gateway_types::MAX_NETWORK
    {
        return Err(denied(
            "provisioned identity, policy or network exceeds its installed record limit",
        ));
    }
    let target = GatewayTarget {
        format: VERSION,
        machine_id: identity.machine_id.clone(),
        public_key: identity.public_key.clone(),
        direct_addresses: Vec::new(),
        relay_urls: request.network.relay_urls.clone(),
        use_default_relays: request.network.use_default_relays,
        allow_plaintext_relays: request.network.allow_plaintext_relays,
    };
    target.validate()?;
    let public = serde_json::to_vec_pretty(&target).map_err(io::Error::other)?;
    if public.len() > crate::login::core::gateway_types::MAX_NETWORK {
        return Err(denied("provisioned public target exceeds its record limit"));
    }
    platform::write_provision_bundle(
        &request.directory,
        &[
            ("machine-key.json", private.as_slice()),
            ("policy.json", &policy_bytes),
            ("gateway-network.json", &network),
            ("gateway-target.json", &public),
        ],
    )?;
    Ok(Provisioned {
        identity_file: request.directory.join("machine-key.json"),
        policy_file: request.directory.join("policy.json"),
        network_file: request.directory.join("gateway-network.json"),
        target_file: request.directory.join("gateway-target.json"),
        target,
    })
}
#[derive(serde::Serialize)]
pub struct ProvisionedNetwork {
    pub network_file: PathBuf,
    pub network: GatewayNetwork,
}
/// Network refresh for an installed gateway (L-B2, E24): writes only
/// `gateway-network.json` with the relays of the configured Share server. No
/// machine identity, policy or target is created, so the installed ones stay.
pub fn provision_network(directory: PathBuf) -> io::Result<ProvisionedNetwork> {
    let network = resolve_network(GatewayNetwork {
        format: VERSION,
        use_default_relays: true,
        relay_urls: Vec::new(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    })?;
    check_location(&directory)?;
    let bytes = serde_json::to_vec_pretty(&network).map_err(io::Error::other)?;
    if bytes.len() > crate::login::core::gateway_types::MAX_NETWORK {
        return Err(denied(
            "provisioned network exceeds its installed record limit",
        ));
    }
    platform::write_provision_bundle(&directory, &[("gateway-network.json", &bytes)])?;
    Ok(ProvisionedNetwork {
        network_file: directory.join("gateway-network.json"),
        network,
    })
}

#[cfg(test)]
#[path = "provision_tests.rs"]
mod tests;
