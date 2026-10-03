use clap::Args;
use serde_json::json;

#[derive(Args)]
#[command(group(clap::ArgGroup::new("access").required(true).args(["write", "read_only"])))]
pub(super) struct WriteArgs {
    #[arg(help = "Device id/name/fingerprint, request id, or exact saved contact name")]
    selector: String,
    #[arg(
        long,
        help = "Allow this already admitted device to write writable exports"
    )]
    write: bool,
    #[arg(long, help = "Withdraw the device's write right, keeping read access")]
    read_only: bool,
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
    let change =
        crate::share::set_direct_peer_write(Some(super::super::default_home()), &peer, args.write)?;
    let (state, error) = super::worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "action": "write_right_changed", "device_id": peer.device_id, "write": args.write,
                "changed": change.changed, "persisted": true,
                "worker_refresh": {"state": state, "error": error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "write_right\t{}\twrite={}\tpersisted=true\tworker_refresh={state}",
            peer.device_id, args.write
        );
        if let Some(error) = error {
            println!("worker_error\t{}", super::clean(&error));
        }
    }
    Ok(())
}
