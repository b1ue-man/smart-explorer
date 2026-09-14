//! `se share grants delete` and `se share grants removed`: delete an
//! authorization entry completely and manage the removed-device denial list.
use super::{clean, grant_selector_matches, worker_refresh, DeleteArgs, RemovedArgs};

pub(super) fn delete(args: DeleteArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let matches = profiles
        .direct_grants
        .iter()
        .filter(|grant| {
            args.selector
                .as_deref()
                .is_none_or(|selector| grant_selector_matches(&profiles, grant, selector.trim()))
        })
        .collect::<Vec<_>>();
    let grant = match matches.as_slice() {
        [] => {
            return Err(match args.selector.as_deref() {
                Some(selector) => format!(
                    "grant not found: {selector}; run `se share grants` to list valid selectors"
                ),
                None => "no grants stored; `se share grants` shows the empty list".to_string(),
            })
        }
        [grant] => *grant,
        _ => {
            return Err(format!(
                "multiple grants; choose one selector shown by `se share grants`: {}",
                matches
                    .iter()
                    .map(|grant| grant.device_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };
    let device_id = grant.device_id.clone();
    let home = Some(super::super::default_home());
    // A contact behind the grant is removed as a whole, exactly like the GUI.
    let contact_id = profiles
        .direct_contacts
        .iter()
        .find(|contact| contact.remote_device_id.as_deref() == Some(device_id.as_str()))
        .map(|contact| contact.id.clone());
    let (action, cleanup) = match contact_id {
        Some(contact_id) => {
            let (_profiles, change, _forgotten) =
                crate::share::ShareProfiles::forget_direct_peer_persisted(home, &contact_id)?;
            if let Some(warning) = change.cleanup_warning {
                return Err(format!("removed device {device_id}, but {warning}"));
            }
            let cleanup = crate::app::cleanup_removed_endpoint_state(
                &crate::app::RemovedEndpointScope::for_direct_contact(&contact_id),
            );
            ("removed_device", cleanup.summary_suffix())
        }
        None => {
            crate::share::ShareProfiles::delete_direct_grant_persisted(home, &device_id)?;
            ("deleted_grant", String::new())
        }
    };
    let (worker_state, worker_error) = worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": action,
                "device_id": device_id,
                "automatic_repair": "blocked until paired again",
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\t{action}\tdevice_id={}\tautomatic_repair=blocked\tworker_refresh={worker_state}{cleanup}",
            clean(&device_id)
        );
        if let Some(error) = worker_error {
            println!("worker_error\t{}", clean(&error));
        }
    }
    Ok(())
}

pub(super) fn removed(args: RemovedArgs, json: bool) -> Result<(), String> {
    if let Some(device_id) = args.readmit.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
        let (_profiles, change) = crate::share::ShareProfiles::readmit_removed_direct_peer_persisted(
            Some(super::super::default_home()),
            device_id,
        )?;
        if !change.changed {
            return Err(format!("removed device not found: {device_id}"));
        }
        let (worker_state, worker_error) = worker_refresh();
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "action": "readmitted",
                    "device_id": device_id,
                    "worker_refresh": {"state": worker_state, "error": worker_error},
                }))
                .map_err(|error| error.to_string())?
            );
        } else {
            println!(
                "action\treadmitted\tdevice_id={}\tworker_refresh={worker_state}",
                clean(device_id)
            );
        }
        return Ok(());
    }
    let profiles = super::super::checked_profiles()?;
    if json {
        let records = profiles
            .removed_direct_peers
            .iter()
            .map(|record| {
                serde_json::json!({
                    "device_id": record.device_id,
                    "device_name": record.device_name,
                    "fingerprint": record.fingerprint,
                    "node_id": record.node_id,
                    "removed_at": record.removed_at,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": records.len(),
                "removed": records,
            }))
            .map_err(|error| error.to_string())?
        );
    } else if profiles.removed_direct_peers.is_empty() {
        println!("removed\t0");
    } else {
        for record in &profiles.removed_direct_peers {
            println!(
                "removed\t{}\tdevice_name={}\tfingerprint={}\tremoved_at={}\treadmit=`se share grants removed --readmit {}`",
                clean(&record.device_id),
                clean(&record.device_name),
                clean(&record.fingerprint),
                record.removed_at,
                clean(&record.device_id)
            );
        }
    }
    Ok(())
}

