use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use super::lifecycle_output;

#[path = "grants_exec.rs"]
mod grants_exec;

#[derive(Args)]
#[command(long_about = "Inspect and revoke direct authorization grants.\n\n\
With no subcommand, this lists every grant and its active/inactive state.\n\
`revoke` needs no selector when exactly one authorization is active.")]
pub(super) struct GrantsArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<GrantsCommand>,
}

#[derive(Subcommand)]
enum GrantsCommand {
    #[command(about = "List local direct authorization grants and linked requests")]
    List,
    #[command(about = "Revoke an authorization; auto-selects the only active grant")]
    Revoke(RevokeArgs),
    #[command(about = "Inspect, enable, or disable unrestricted per-device Exec grants")]
    Exec(grants_exec::ExecGrantArgs),
}

#[derive(Args)]
struct RevokeArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional request/device/name/fingerprint selector; omit when only one grant is active",
        add = ArgValueCandidates::new(crate::cli::completions::active_grant_candidates)
    )]
    selector: Option<String>,
    #[arg(long, help = "Optional extra fingerprint assertion")]
    fingerprint: Option<String>,
    #[arg(long, help = "Optional signed revocation message")]
    message: Option<String>,
}

pub(super) fn run(args: GrantsArgs) -> Result<(), String> {
    match args.command {
        None | Some(GrantsCommand::List) => list(args.json),
        Some(GrantsCommand::Revoke(command)) => revoke(command, args.json),
        Some(GrantsCommand::Exec(command)) => grants_exec::run(command, args.json),
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
                "selector": grant.device_id,
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
                "exec": {
                    "enabled": grant.exec.enabled,
                    "policy_revision": grant.exec.policy_revision,
                    "changed_at": grant.exec.changed_at,
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
            "grant\t{}\tdevice_name={}\tfingerprint={}\tstate={}\tupdated_at={}\tauthorization={}\texec={}\texec_revision={}\tconnectivity=unknown\trequest_ids={}",
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
            if grant.exec.enabled { "enabled" } else { "disabled" },
            grant.exec.policy_revision,
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

fn revoke(args: RevokeArgs, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let matches = profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Accepted)
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
                "active grant not found: {selector}; run `se share grants` to list valid selectors"
            ),
                None => "no active grants; `se share grants` shows accepted and inactive grants"
                    .to_string(),
            })
        }
        [grant] => *grant,
        _ => {
            return Err(format!(
                "multiple active grants; choose one selector shown by `se share grants`: {}",
                matches
                    .iter()
                    .map(|grant| grant.device_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };
    super::request_selection::verify_optional_fingerprint(
        args.fingerprint.as_deref(),
        &grant.fingerprint,
        &grant.device_id,
    )?;
    let Some(entry) = latest_accepted_request(&profiles, &grant.device_id) else {
        return revoke_legacy(grant, json);
    };
    let request_id = entry.record.request.request_id.clone();
    let identity = super::identity_command::load_with_repair_hint()?;
    let persisted = crate::share::decide_direct_request(
        Some(super::default_home()),
        &identity,
        &request_id,
        &grant.fingerprint,
        crate::share::DirectDecisionKind::Revoked,
        args.message,
    )?;
    let (worker_state, worker_error) = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .direct_request(&request_id)
        .cloned()
        .unwrap_or(persisted);
    if json {
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

fn revoke_legacy(grant: &crate::share::DirectGrant, json: bool) -> Result<(), String> {
    let device_id = grant.device_id.clone();
    let public_key = grant.public_key.clone();
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
            current.exec.disable_without_decision(now);
            Ok(())
        })?;
    let persisted = committed
        .direct_grants
        .iter()
        .find(|current| current.device_id == device_id)
        .ok_or_else(|| format!("persisted legacy grant is missing: {device_id}"))?;
    let (worker_state, worker_error) = worker_refresh();
    if json {
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

fn latest_accepted_request<'a>(
    profiles: &'a crate::share::ShareProfiles,
    device_id: &'a str,
) -> Option<&'a crate::share::DirectRequestEntry> {
    related_requests(profiles, device_id)
        .filter(|entry| entry.record.decision.state == crate::share::DirectDecisionState::Accepted)
        .max_by_key(|entry| entry.record.decision.changed_at)
}

fn grant_selector_matches(
    profiles: &crate::share::ShareProfiles,
    grant: &crate::share::DirectGrant,
    selector: &str,
) -> bool {
    grant.device_name.eq_ignore_ascii_case(selector)
        || exact_or_prefix(selector, &grant.device_id)
        || exact_or_prefix(selector, &grant.fingerprint)
        || related_requests(profiles, &grant.device_id)
            .any(|entry| exact_or_prefix(selector, entry.record.request.request_id.as_str()))
}

fn exact_or_prefix(selector: &str, candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case(selector)
        || (selector.len() >= 4
            && candidate
                .to_ascii_lowercase()
                .starts_with(&selector.to_ascii_lowercase()))
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
