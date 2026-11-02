use clap::Args;
use serde_json::json;

use super::super::rights_args::{rights_value, RightsArgs};

#[derive(Args)]
pub(super) struct WriteArgs {
    #[arg(help = "Device id/name/fingerprint, request id, or exact saved contact name")]
    selector: String,
    // --write / --read-only alone change only the write right (as before);
    // a --preset or any other option sets exactly the resulting rights.
    #[command(flatten)]
    rights: RightsArgs,
}

pub(super) fn run(args: WriteArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let selector = args.selector.trim();
    if selector.is_empty() {
        return Err("contact selector must not be empty".into());
    }
    let matches = profiles
        .direct_grants
        .iter()
        .filter(|grant| {
            super::grant_selector_matches(&profiles, grant, selector)
                || profiles.direct_contacts.iter().any(|contact| {
                    contact.display_name.eq_ignore_ascii_case(selector)
                        && crate::share::ShareProfiles::contact_remote_identity(contact)
                            .is_some_and(|peer| {
                                peer.device_id == grant.device_id
                                    && peer.public_key == grant.public_key
                                    && peer.fingerprint == grant.fingerprint
                                    && (peer.node_id.is_empty() || peer.node_id == grant.node_id)
                            })
                })
        })
        .collect::<Vec<_>>();
    let [grant] = matches.as_slice() else {
        return Err(
            "contact authorization is missing or ambiguous; use the device id from grants list"
                .into(),
        );
    };
    let peer = crate::share::DirectPeerIdentity {
        device_id: grant.device_id.clone(),
        device_name: grant.device_name.clone(),
        public_key: grant.public_key.clone(),
        node_id: grant.node_id.clone(),
        fingerprint: grant.fingerprint.clone(),
    };
    if !args.rights.given() {
        return Err("choose --write, --read-only, --preset NAME or a single rights option".into());
    }
    if !args.rights.only_write() {
        return set_rights(&profiles, &peer, &args.rights, json);
    }
    let write = args.rights.write;
    let change =
        crate::share::set_direct_peer_write(Some(super::super::default_home()), &peer, write)?;
    let (state, error) = super::worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "action": "write_right_changed", "device_id": peer.device_id, "write": write,
                "changed": change.changed, "persisted": true,
                "worker_refresh": {"state": state, "error": error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "write_right\t{}\twrite={}\tpersisted=true\tworker_refresh={state}",
            peer.device_id, write
        );
        if let Some(error) = error {
            println!("worker_error\t{}", super::clean(&error));
        }
    }
    Ok(())
}

/// F3/F4: exactly the rights of the preset and options for the device, the
/// current rights where the options name nothing (one transaction).
fn set_rights(
    profiles: &crate::share::ShareProfiles,
    peer: &crate::share::DirectPeerIdentity,
    args: &RightsArgs,
    json: bool,
) -> Result<(), String> {
    let now = crate::share::core_now_secs();
    let base = profiles.current_direct_rights(&peer.device_id, &peer.public_key);
    let roots = &profiles.default_direct_exports.roots;
    let rights = args.rights(base, roots, now)?.ok_or("choose the rights")?;
    let target = crate::share::connection_rights::RightsTarget::Direct {
        device_id: peer.device_id.clone(),
        public_key: peer.public_key.clone(),
    };
    let change = crate::share::connection_rights_actions::confirm_connection_rights(
        Some(super::super::default_home()),
        &target,
        &rights,
    )?;
    let (state, error) = super::worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "action": "rights_changed", "device_id": peer.device_id,
                "rights": rights_value(&rights), "changed": change.changed, "persisted": true,
                "worker_refresh": {"state": state, "error": error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "rights\t{}\t{}\tpersisted=true\tworker_refresh={state}",
            peer.device_id,
            rights.summary()
        );
        if let Some(error) = error {
            println!("worker_error\t{}", super::clean(&error));
        }
    }
    Ok(())
}
