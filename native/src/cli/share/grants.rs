use clap::{Args, Subcommand};

use super::lifecycle_output;

#[derive(Args)]
pub(super) struct GrantsArgs {
    #[command(subcommand)]
    command: GrantsCommand,
}

#[derive(Subcommand)]
enum GrantsCommand {
    #[command(about = "List local direct authorization grants and linked requests")]
    List(JsonArgs),
    #[command(about = "Sign and persist a revisioned grant revocation")]
    Revoke(RevokeArgs),
}

#[derive(Args)]
struct JsonArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct RevokeArgs {
    #[arg(help = "Accepted incoming request UUID or exact requester device ID")]
    selector: String,
    #[arg(long, help = "Exact requester fingerprint shown by grants list")]
    fingerprint: String,
    #[arg(long, help = "Optional signed revocation message")]
    message: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: GrantsArgs) -> Result<(), String> {
    match args.command {
        GrantsCommand::List(args) => list(args.json),
        GrantsCommand::Revoke(args) => revoke(args),
    }
}

pub(super) fn values(profiles: &crate::share::ShareProfiles) -> Vec<serde_json::Value> {
    profiles
        .direct_grants
        .iter()
        .map(|grant| {
            let requests = related_requests(profiles, &grant.device_id)
                .map(|entry| lifecycle_output::request_value(entry, profiles))
                .collect::<Vec<_>>();
            serde_json::json!({
                "device_id": grant.device_id,
                "device_name": grant.device_name,
                "node_id": grant.node_id,
                "public_key": grant.public_key,
                "fingerprint": grant.fingerprint,
                "grant_state": grant_state_code(&grant.state),
                "updated_at": grant.updated_at,
                "authorization": {
                    "state": if grant.state == crate::share::DirectGrantState::Accepted {
                        "active"
                    } else {
                        "inactive"
                    },
                    "active": grant.state == crate::share::DirectGrantState::Accepted,
                },
                "connectivity": {"state": "unknown", "label": "Not tracked per grant"},
                "tracked": !requests.is_empty(),
                "requests": requests,
            })
        })
        .collect()
}

pub(super) fn text(profiles: &crate::share::ShareProfiles) -> Vec<String> {
    let mut lines = Vec::new();
    for grant in &profiles.direct_grants {
        let related = related_requests(profiles, &grant.device_id).collect::<Vec<_>>();
        let request_ids = related
            .iter()
            .map(|entry| entry.record.request.request_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!(
            "grant\t{}\tdevice_name={}\tfingerprint={}\tstate={}\tupdated_at={}\tauthorization={}\tconnectivity=unknown\trequest_ids={}",
            clean(&grant.device_id),
            clean(&grant.device_name),
            clean(&grant.fingerprint),
            grant_state_code(&grant.state),
            grant.updated_at,
            if grant.state == crate::share::DirectGrantState::Accepted {
                "active"
            } else {
                "inactive"
            },
            request_ids,
        ));
        for entry in related {
            lines.extend(lifecycle_output::request_text(entry, profiles));
        }
    }
    lines
}

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    if json {
        let grants = values(&profiles);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": grants.len(),
                "grants": grants,
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        let lines = text(&profiles);
        if lines.is_empty() {
            println!("grants\t0");
        } else {
            for line in lines {
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn revoke(args: RevokeArgs) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let mut matches = profiles
        .direct_requests
        .iter()
        .filter(|entry| {
            entry.direction == crate::share::DirectRequestDirection::Incoming
                && entry.decision.as_ref().is_some_and(|decision| {
                    decision.decision == crate::share::DirectDecisionKind::Accepted
                })
                && (entry.record.request.request_id.as_str() == args.selector.trim()
                    || entry.record.request.requester.device_id == args.selector.trim())
        })
        .cloned()
        .collect::<Vec<_>>();
    if matches.is_empty() {
        if let Some(grant) = profiles.direct_grants.iter().find(|grant| {
            grant.device_id == args.selector.trim()
                && grant.state == crate::share::DirectGrantState::Accepted
        }) {
            return revoke_legacy(
                &args,
                grant.device_id.clone(),
                grant.public_key.clone(),
                grant.fingerprint.clone(),
            );
        }
    }
    let entry = match matches.len() {
        0 => return Err(format!("active tracked grant not found: {}", args.selector)),
        1 => matches.remove(0),
        _ => {
            return Err(format!(
                "grant selector is ambiguous; use a request UUID: {}",
                matches
                    .iter()
                    .map(|entry| entry.record.request.request_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };
    let request_id = entry.record.request.request_id.clone();
    let identity = super::identity_command::load_with_repair_hint()?;
    let persisted = crate::share::decide_direct_request(
        Some(super::default_home()),
        &identity,
        &request_id,
        &args.fingerprint,
        crate::share::DirectDecisionKind::Revoked,
        args.message,
    )?;
    let (worker_state, worker_error) = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .direct_request(&request_id)
        .cloned()
        .unwrap_or(persisted);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "revoked",
                "request": lifecycle_output::request_value(&entry, &committed),
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\trevoked\trequest_id={}\tpersisted=true\tworker_refresh={}",
            request_id, worker_state
        );
        if let Some(error) = worker_error {
            println!("worker_error\t{}", clean(&error));
        }
        lifecycle_output::print_request(&entry, &committed, false);
    }
    Ok(())
}

fn revoke_legacy(
    args: &RevokeArgs,
    device_id: String,
    public_key: String,
    fingerprint: String,
) -> Result<(), String> {
    if !fingerprint.eq_ignore_ascii_case(args.fingerprint.trim()) {
        return Err(format!(
            "fingerprint mismatch for {}: expected {}",
            device_id, fingerprint
        ));
    }
    let now = crate::share::core_now_secs();
    let committed =
        crate::share::ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
            let current = profiles
                .direct_grants
                .iter_mut()
                .find(|current| current.device_id == device_id && current.public_key == public_key)
                .ok_or_else(|| format!("legacy grant disappeared: {device_id}"))?;
            current.state = crate::share::DirectGrantState::Ignored;
            current.updated_at = now;
            Ok(())
        })?;
    let persisted = committed
        .direct_grants
        .iter()
        .find(|current| current.device_id == device_id)
        .ok_or_else(|| format!("persisted legacy grant is missing: {device_id}"))?;
    let (worker_state, worker_error) = worker_refresh();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "revoked",
                "legacy": true,
                "device_id": persisted.device_id,
                "fingerprint": persisted.fingerprint,
                "decision": "revoked",
                "decision_delivery": "local_only_untracked",
                "authorization": {"state": "inactive", "active": false},
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "legacy_grant\t{}\tdecision=revoked\tdecision_delivery=local_only_untracked\tauthorization=inactive\tpersisted=true\tworker_refresh={}",
            clean(&persisted.device_id), worker_state
        );
        if let Some(error) = worker_error {
            println!("worker_error\t{}", clean(&error));
        }
    }
    Ok(())
}

fn related_requests<'a>(
    profiles: &'a crate::share::ShareProfiles,
    device_id: &'a str,
) -> impl Iterator<Item = &'a crate::share::DirectRequestEntry> {
    profiles.direct_requests.iter().filter(move |entry| {
        entry.direction == crate::share::DirectRequestDirection::Incoming
            && entry.record.request.requester.device_id == device_id
    })
}

fn worker_refresh() -> (&'static str, Option<String>) {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => ("refreshed", None),
        Ok(false) => (
            "inactive",
            Some("Share server is not configured or Auto-Connect is off".to_string()),
        ),
        Err(error) => ("unavailable", Some(error)),
    }
}

fn grant_state_code(state: &crate::share::DirectGrantState) -> &'static str {
    match state {
        crate::share::DirectGrantState::Accepted => "accepted",
        crate::share::DirectGrantState::Ignored => "ignored",
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
