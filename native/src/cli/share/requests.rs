use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use super::lifecycle_output;
use super::request_selection::{
    ambiguous_pending_error, is_pending_incoming, matching_legacy, matching_tracked,
    no_pending_error, pending_legacy, pending_tracked, select_tracked, verify_optional_fingerprint,
};

#[derive(Args)]
#[command(long_about = "Inspect and decide durable direct access requests.\n\n\
With no subcommand, this shows the pending incoming inbox and the exact next\n\
command. `accept` and `reject` need no selector when exactly one request is\n\
pending; `show`, `retry`, and `delete` likewise auto-select their sole eligible\n\
entry. Selectors can be copied from this command's output or completed with\n\
the shell integration from `se completions <shell>`.")]
pub(super) struct RequestArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<RequestCommand>,
}

#[derive(Subcommand)]
enum RequestCommand {
    #[command(about = "List durable outgoing and incoming direct requests")]
    List,
    #[command(about = "Show one durable request by any selector emitted by request list")]
    Show(ShowArgs),
    #[command(about = "Accept a pending incoming request; auto-selects the only one")]
    Accept(DecisionArgs),
    #[command(about = "Reject a pending incoming request; auto-selects the only one")]
    Reject(DecisionArgs),
    #[command(about = "Retry the pending envelope for the same request ID now")]
    Retry(SelectionArgs),
    #[command(about = "Delete a request locally, stop retries, and retain a replay tombstone")]
    Delete(SelectionArgs),
}

#[derive(Args)]
struct ShowArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional request UUID, device ID/name, fingerprint, or prefix; omit when only one exists",
        add = ArgValueCandidates::new(crate::cli::completions::request_candidates)
    )]
    selector: Option<String>,
}

#[derive(Args)]
struct DecisionArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional request UUID, device ID/name, fingerprint, or prefix; omit when only one is pending",
        add = ArgValueCandidates::new(crate::cli::completions::pending_request_candidates)
    )]
    selector: Option<String>,
    #[arg(
        long,
        help = "Optional extra fingerprint assertion; never required for a stored signed request"
    )]
    fingerprint: Option<String>,
    #[arg(long, help = "Optional signed decision message")]
    message: Option<String>,
}

#[derive(Args)]
struct SelectionArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional request selector shown by request list; omit when only one is eligible",
        add = ArgValueCandidates::new(crate::cli::completions::request_candidates)
    )]
    selector: Option<String>,
}

pub(super) fn run(args: RequestArgs) -> Result<(), String> {
    match args.command {
        None => inbox(args.json),
        Some(RequestCommand::List) => list(args.json),
        Some(RequestCommand::Show(command)) => show(command.selector.as_deref(), args.json),
        Some(RequestCommand::Accept(command)) => decide(
            command,
            crate::share::DirectDecisionKind::Accepted,
            args.json,
        ),
        Some(RequestCommand::Reject(command)) => decide(
            command,
            crate::share::DirectDecisionKind::Rejected,
            args.json,
        ),
        Some(RequestCommand::Retry(command)) => retry(command.selector.as_deref(), args.json),
        Some(RequestCommand::Delete(command)) => {
            delete_history(command.selector.as_deref(), args.json)
        }
    }
}

fn inbox(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let tracked = pending_tracked(&profiles, now);
    let (legacy, worker_error) = match crate::daemon::drain_share_worker_events() {
        Ok(snapshot) => (pending_legacy(snapshot.pending_direct_requests, now), None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let count = tracked.len() + legacy.len();
    let history_count = profiles.direct_requests.len();
    if json {
        let requests = tracked
            .iter()
            .map(|entry| lifecycle_output::request_value(entry, &profiles))
            .collect::<Vec<_>>();
        let legacy_requests = legacy.iter().map(legacy_value).collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": count,
                "requests": requests,
                "legacy_requests": legacy_requests,
                "worker_error": worker_error,
                "next_command": (count == 1).then_some("se share request accept"),
                "history_count": history_count,
                "history_command": "se share request list",
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!("pending_requests\t{count}");
    println!("request_history\t{history_count}");
    for entry in &tracked {
        let request = &entry.record.request;
        println!(
            "pending_request\t{}\tdevice_name={}\tdevice_id={}\tfingerprint={}\tdelivery={}\tdecision={}\tauthorization=inactive",
            request.request_id,
            clean(&request.requester.device_name),
            clean(&request.requester.device_id),
            clean(&request.requester.fingerprint),
            entry.record.delivery.state.code(),
            entry.record.decision.state.code(),
        );
    }
    for request in &legacy {
        println!(
            "pending_legacy_request\t{}\tdevice_name={}\tfingerprint={}\tdelivery=received\tdecision=pending\tauthorization=inactive",
            clean(&request.device_id),
            clean(&request.device_name),
            clean(&request.fingerprint),
        );
    }
    if count == 1 {
        println!("next\tse share request accept");
    } else if count > 1 {
        for entry in &tracked {
            println!(
                "accept\tse share request accept {}",
                entry.record.request.request_id
            );
        }
        for request in &legacy {
            println!(
                "accept\tse share request accept {}",
                clean(&request.device_id)
            );
        }
    }
    println!("history\tse share request list");
    if let Some(error) = worker_error {
        println!("worker_error\t{}", clean(&error));
    }
    Ok(())
}

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    if json {
        let requests = profiles
            .direct_requests
            .iter()
            .map(|entry| lifecycle_output::request_value(entry, &profiles))
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": requests.len(),
                "requests": requests,
            }))
            .map_err(|error| error.to_string())?
        );
    } else if profiles.direct_requests.is_empty() {
        println!("requests\t0");
    } else {
        for entry in &profiles.direct_requests {
            for line in lifecycle_output::request_text(entry, &profiles) {
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn show(selector: Option<&str>, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let entry = select_tracked(&profiles, selector, |_| true, "request")?;
    lifecycle_output::print_request(entry, &profiles, json);
    Ok(())
}

fn decide(
    args: DecisionArgs,
    decision: crate::share::DirectDecisionKind,
    json: bool,
) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let tracked_matches = matching_tracked(&profiles, args.selector.as_deref(), |entry| {
        is_pending_incoming(entry, now)
    });
    let legacy_snapshot = crate::daemon::drain_share_worker_events().ok();
    let legacy = legacy_snapshot
        .map(|snapshot| pending_legacy(snapshot.pending_direct_requests, now))
        .unwrap_or_default();
    let legacy_matches = matching_legacy(&legacy, args.selector.as_deref());
    match tracked_matches.len() + legacy_matches.len() {
        0 => {
            return Err(no_pending_error(
                args.selector.as_deref(),
                &profiles,
                &legacy,
                now,
            ))
        }
        1 if tracked_matches.len() == 1 => {}
        1 => {
            return answer_legacy(
                args,
                legacy_matches[0].clone(),
                decision == crate::share::DirectDecisionKind::Accepted,
                json,
            )
        }
        _ => {
            return Err(ambiguous_pending_error(&tracked_matches, &legacy_matches));
        }
    }
    let entry = tracked_matches[0];
    let request_id = entry.record.request.request_id.clone();
    let expected_fingerprint = &entry.record.request.requester.fingerprint;
    verify_optional_fingerprint(
        args.fingerprint.as_deref(),
        expected_fingerprint,
        &request_id,
    )?;
    let identity = super::identity_command::load_with_repair_hint()?;
    let persisted = crate::share::decide_direct_request(
        Some(super::default_home()),
        &identity,
        &request_id,
        expected_fingerprint,
        decision,
        args.message,
    )?;
    let worker = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .direct_request(&request_id)
        .cloned()
        .unwrap_or(persisted);
    print_action(&entry, &committed, decision.code(), worker, json)
}

fn retry(selector: Option<&str>, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let request_id = select_tracked(
        &profiles,
        selector,
        |entry| !entry.pending_outboxes(now).is_empty(),
        "retryable request",
    )?
    .record
    .request
    .request_id
    .clone();
    let persisted =
        crate::share::retry_direct_request_now(Some(super::default_home()), &request_id)?;
    let worker = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .direct_request(&request_id)
        .cloned()
        .unwrap_or(persisted);
    print_action(&entry, &committed, "retry_due_now", worker, json)
}

fn delete_history(selector: Option<&str>, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let entry = select_tracked(&profiles, selector, |_| true, "request")?;
    let request_id = entry.record.request.request_id.clone();
    crate::share::delete_direct_request_history(Some(super::default_home()), &request_id)?;
    let worker = worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "deleted",
                "request_id": request_id.as_str(),
                "persisted": true,
                "worker_refresh": worker.value(),
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\tdeleted\trequest_id={request_id}\tpersisted=true\tworker_refresh={}",
            worker.state
        );
    }
    Ok(())
}

fn print_action(
    entry: &crate::share::DirectRequestEntry,
    profiles: &crate::share::ShareProfiles,
    action: &str,
    worker: WorkerRefresh,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": action,
                "request": lifecycle_output::request_value(entry, profiles),
                "worker_refresh": worker.value(),
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\t{}\trequest_id={}\tpersisted=true\tworker_refresh={}",
            action, entry.record.request.request_id, worker.state
        );
        if let Some(error) = &worker.error {
            println!("worker_error\t{}", clean(error));
        }
        lifecycle_output::print_request(entry, profiles, false);
    }
    Ok(())
}

fn answer_legacy(
    args: DecisionArgs,
    request: crate::share::PeerPresence,
    accepted: bool,
    json: bool,
) -> Result<(), String> {
    verify_optional_fingerprint(
        args.fingerprint.as_deref(),
        &request.fingerprint,
        &request.device_id,
    )?;
    let identity = super::identity_command::load_with_repair_hint()?;
    let state = if accepted {
        crate::share::DirectGrantState::Accepted
    } else {
        crate::share::DirectGrantState::Ignored
    };
    crate::share::ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        profiles.set_direct_grant(&request, state.clone());
        Ok(())
    })?;
    let device_id = request.device_id.clone();
    crate::daemon::send_share_command(crate::share::ShareCmd::AnswerDirectRequest {
        lookup_id: identity.direct_lookup_id,
        presence: request,
        accepted,
    })
    .map_err(|error| format!("legacy decision persisted, but delivery failed: {error}"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "legacy": true,
                "device_id": device_id,
                "decision": if accepted { "accepted" } else { "rejected" },
                "decision_delivery": "attempted_untracked",
                "authorization": if accepted { "active" } else { "inactive" },
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "legacy_request\t{}\tdecision={}\tdecision_delivery=attempted_untracked\tauthorization={}",
            device_id,
            if accepted { "accepted" } else { "rejected" },
            if accepted { "active" } else { "inactive" },
        );
    }
    Ok(())
}

fn legacy_value(request: &crate::share::PeerPresence) -> serde_json::Value {
    serde_json::json!({
        "legacy": true,
        "selector": request.device_id,
        "device_id": request.device_id,
        "device_name": request.device_name,
        "fingerprint": request.fingerprint,
        "expires_at": request.expires_at,
        "delivery": {"state": "received"},
        "decision": {"state": "pending"},
        "authorization": {"state": "inactive", "active": false},
    })
}

struct WorkerRefresh {
    state: &'static str,
    error: Option<String>,
}

impl WorkerRefresh {
    fn value(&self) -> serde_json::Value {
        serde_json::json!({"state": self.state, "error": self.error})
    }
}

fn worker_refresh() -> WorkerRefresh {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => WorkerRefresh {
            state: "refreshed",
            error: None,
        },
        Ok(false) => WorkerRefresh {
            state: "inactive",
            error: Some("Share server is not configured or Auto-Connect is off".to_string()),
        },
        Err(error) => WorkerRefresh {
            state: "unavailable",
            error: Some(error),
        },
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
