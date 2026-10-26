//! Read-only identity selection and exact per-relationship policy sources.
//! This view never combines grants for authorization or changes a policy.
use std::collections::BTreeMap;

use super::{clean, output, print};

pub(super) fn show(
    profiles: &crate::share::ShareProfiles,
    selector: Option<&str>,
    json: bool,
) -> Result<(), String> {
    let all = crate::share::exec_target_views(profiles);
    let mut identities = BTreeMap::new();
    for view in &all {
        let matches = selector.is_none_or(|selector| {
            let prefix = |value: &str| {
                value.eq_ignore_ascii_case(selector)
                    || (selector.len() >= 4
                        && value
                            .to_ascii_lowercase()
                            .starts_with(&selector.to_ascii_lowercase()))
            };
            view.device_name.eq_ignore_ascii_case(selector)
                || prefix(&view.device_id)
                || prefix(&view.fingerprint)
                || output::selector(&view.target) == selector
        });
        if !matches {
            continue;
        }
        let identity = match &view.target {
            crate::share::ExecGrantTarget::Direct {
                device_id,
                public_key,
                ..
            }
            | crate::share::ExecGrantTarget::RoomMember {
                device_id,
                public_key,
                ..
            } => (device_id.clone(), public_key.clone()),
        };
        identities.entry(identity).or_insert(view);
    }
    if identities.len() != 1 {
        return Err("name exactly one pinned device by name, device id, fingerprint or exec:// endpoint; equal names need an identity selector".into());
    }
    let ((device, key), target) = identities
        .iter()
        .next()
        .ok_or("device selection is empty")?;
    crate::daemon::ensure_worker_ready()?;
    let rights = crate::daemon::access_ipc::device_access_rights(device, key)?;
    let value = serde_json::json!({ "device_id": device, "device_name": target.device_name, "fingerprint": target.fingerprint, "device_access": rights });
    let mut lines = vec![format!(
        "device_access\t{}\t{}",
        clean(device),
        clean(&target.device_name)
    )];
    for source in &rights.relations {
        lines.push(format!(
            "relationship\t{}\tactive={}\tpolicy={}",
            clean(&source.relation_name),
            source.active,
            serde_json::to_string(source).map_err(|error| error.to_string())?
        ));
    }
    print(json, &value, &lines)
}
