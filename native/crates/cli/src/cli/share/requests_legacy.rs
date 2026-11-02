pub(super) fn value(
    entry: &crate::share::LegacyDirectRequestEntry,
    profiles: &crate::share::ShareProfiles,
) -> serde_json::Value {
    let active = entry.authorization_active(profiles);
    let resolution_commands =
        super::request_selection::legacy_conflict_resolution_commands(profiles, entry);
    serde_json::json!({
        "legacy": true,
        "protocol": "legacy",
        "tracked": false,
        "request_id_scope": "local",
        "evidence": "hmac_relation",
        "selector": entry.selector,
        "device_id": entry.peer.device_id,
        "device_name": entry.peer.device_name,
        "fingerprint": entry.peer.fingerprint,
        "node_id": entry.peer.node_id,
        "received_at": entry.first_received_at,
        "last_received_at": entry.last_received_at,
        "expires_at": entry.evidence.expires_at,
        "identity_conflict": entry.identity_conflict,
        "resolution_commands": resolution_commands,
        "delivery": {"state": "received", "scope": "local_persisted"},
        "decision": {
            "state": entry.decision.code(),
            "revision": entry.decision_revision,
            "delivery": {
                "channel": entry.decision_delivery_channel(),
                "state": entry.decision_delivery.state.code(),
                "attempt_count": entry.decision_delivery.attempt_count,
                "last_attempt_at": entry.decision_delivery.last_attempt_at,
                "last_error": entry.decision_delivery.last_error,
            },
        },
        "authorization": {
            "state": if active { "active" } else { "inactive" },
            "active": active,
        },
    })
}

pub(super) fn text(
    entry: &crate::share::LegacyDirectRequestEntry,
    profiles: &crate::share::ShareProfiles,
) -> String {
    let mut output = format!(
        "legacy_request\t{}\tdevice_name={}\tdevice_id={}\tfingerprint={}\tdelivery=received\tdelivery_scope=local_persisted\treceipt=unsupported\tdecision={}\tdecision_channel={}\tdecision_delivery={}\tauthorization={}\tidentity_conflict={}",
        clean(&entry.selector),
        clean(&entry.peer.device_name),
        clean(&entry.peer.device_id),
        clean(&entry.peer.fingerprint),
        entry.decision.code(),
        entry.decision_delivery_channel(),
        entry.decision_delivery.state.code(),
        if entry.authorization_active(profiles) { "active" } else { "inactive" },
        entry.identity_conflict,
    );
    for command in super::request_selection::legacy_conflict_resolution_commands(profiles, entry) {
        output.push_str(&format!(
            "\nlegacy_request_resolution\t{}\t{}",
            clean(&entry.selector),
            clean(&command)
        ));
    }
    output
}

pub(super) fn print(
    entry: &crate::share::LegacyDirectRequestEntry,
    profiles: &crate::share::ShareProfiles,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value(entry, profiles))
                .map_err(|error| error.to_string())?
        );
    } else {
        println!("{}", text(entry, profiles));
    }
    Ok(())
}

pub(super) fn print_action(
    entry: &crate::share::LegacyDirectRequestEntry,
    profiles: &crate::share::ShareProfiles,
    action: &str,
    worker: super::requests_support::WorkerRefresh,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": action,
                "legacy": true,
                "request": value(entry, profiles),
                "worker_refresh": worker.value(),
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\t{}\tselector={}\tpersisted=true\tworker_refresh={}",
            action, entry.selector, worker.state
        );
        if let Some(error) = &worker.error {
            println!("worker_error\t{}", clean(error));
        }
        println!("{}", text(entry, profiles));
    }
    Ok(())
}

pub(super) fn print_deleted(
    key: &str,
    value: &str,
    legacy: bool,
    worker: super::requests_support::WorkerRefresh,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "deleted",
                key: value,
                "legacy": legacy,
                "persisted": true,
                "worker_refresh": worker.value(),
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\tdeleted\t{key}={value}\tlegacy={legacy}\tpersisted=true\tworker_refresh={}",
            worker.state
        );
    }
    Ok(())
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
