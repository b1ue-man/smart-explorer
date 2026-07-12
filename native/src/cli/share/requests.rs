use clap::{Args, Subcommand};

use super::lifecycle_output;

#[derive(Args)]
pub(super) struct RequestArgs {
    #[command(subcommand)]
    command: RequestCommand,
}

#[derive(Subcommand)]
enum RequestCommand {
    #[command(about = "List durable outgoing and incoming direct requests")]
    List(JsonArgs),
    #[command(about = "Show one durable request by stable request ID")]
    Show(ShowArgs),
    #[command(about = "Accept a tracked incoming request")]
    Accept(DecisionArgs),
    #[command(about = "Reject a tracked incoming request")]
    Reject(DecisionArgs),
    #[command(about = "Retry the pending envelope for the same request ID now")]
    Retry(RetryArgs),
}

#[derive(Args)]
struct JsonArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct ShowArgs {
    #[arg(help = "Stable request UUID shown by `se share request list`")]
    request_id: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct DecisionArgs {
    #[arg(help = "Stable request UUID, or a legacy pending device ID")]
    request_id: String,
    #[arg(long, help = "Exact requester fingerprint shown by request list/show")]
    fingerprint: String,
    #[arg(long, help = "Optional signed decision message")]
    message: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct RetryArgs {
    #[arg(help = "Stable request UUID to retry without creating a new request")]
    request_id: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: RequestArgs) -> Result<(), String> {
    match args.command {
        RequestCommand::List(args) => list(args.json),
        RequestCommand::Show(args) => show(&args.request_id, args.json),
        RequestCommand::Accept(args) => decide(args, crate::share::DirectDecisionKind::Accepted),
        RequestCommand::Reject(args) => decide(args, crate::share::DirectDecisionKind::Rejected),
        RequestCommand::Retry(args) => retry(&args.request_id, args.json),
    }
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

fn show(raw_request_id: &str, json: bool) -> Result<(), String> {
    let request_id = parse_request_id(raw_request_id)?;
    let profiles = super::checked_profiles()?;
    let entry = profiles
        .direct_request(&request_id)
        .ok_or_else(|| format!("direct request not found: {request_id}"))?;
    lifecycle_output::print_request(entry, &profiles, json);
    Ok(())
}

fn decide(args: DecisionArgs, decision: crate::share::DirectDecisionKind) -> Result<(), String> {
    let parsed = crate::share::DirectRequestId::parse(args.request_id.trim());
    let profiles = super::checked_profiles()?;
    let tracked = parsed
        .as_ref()
        .ok()
        .is_some_and(|request_id| profiles.direct_request(request_id).is_some());
    if !tracked {
        return answer_legacy(args, decision == crate::share::DirectDecisionKind::Accepted);
    }
    let request_id = parsed.map_err(|error| error.to_string())?;
    let identity = super::identity_command::load_with_repair_hint()?;
    let persisted = crate::share::decide_direct_request(
        Some(super::default_home()),
        &identity,
        &request_id,
        &args.fingerprint,
        decision,
        args.message,
    )?;
    let worker = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .direct_request(&request_id)
        .cloned()
        .unwrap_or(persisted);
    print_action(&entry, &committed, decision.code(), worker, args.json)
}

fn retry(raw_request_id: &str, json: bool) -> Result<(), String> {
    let request_id = parse_request_id(raw_request_id)?;
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

fn answer_legacy(args: DecisionArgs, accepted: bool) -> Result<(), String> {
    let snapshot = crate::daemon::drain_share_worker_events()?;
    let mut matches = snapshot
        .pending_direct_requests
        .into_iter()
        .filter(|request| request.device_id == args.request_id)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(if matches.is_empty() {
            format!(
                "tracked or legacy pending request not found: {}",
                args.request_id
            )
        } else {
            format!(
                "legacy pending request id is ambiguous: {}",
                args.request_id
            )
        });
    }
    let request = matches.remove(0);
    if request.expires_at < crate::share::core_now_secs() {
        return Err(format!(
            "legacy pending request expired: {}",
            args.request_id
        ));
    }
    if !request
        .fingerprint
        .eq_ignore_ascii_case(args.fingerprint.trim())
    {
        return Err(format!(
            "fingerprint mismatch for {}: expected {}",
            args.request_id, request.fingerprint
        ));
    }
    let identity = super::identity_command::load_with_repair_hint()?;
    let mut profiles = super::checked_profiles()?;
    let state = if accepted {
        crate::share::DirectGrantState::Accepted
    } else {
        crate::share::DirectGrantState::Ignored
    };
    profiles.set_direct_grant_persisted(&request, state)?;
    crate::daemon::send_share_command(crate::share::ShareCmd::AnswerDirectRequest {
        lookup_id: identity.direct_lookup_id,
        presence: request,
        accepted,
    })
    .map_err(|error| format!("legacy decision persisted, but delivery failed: {error}"))?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "legacy": true,
                "device_id": args.request_id,
                "decision": if accepted { "accepted" } else { "rejected" },
                "decision_delivery": "attempted_untracked",
                "authorization": if accepted { "active" } else { "inactive" },
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "legacy_request\t{}\tdecision={}\tdecision_delivery=attempted_untracked\tauthorization={}",
            args.request_id,
            if accepted { "accepted" } else { "rejected" },
            if accepted { "active" } else { "inactive" },
        );
    }
    Ok(())
}

fn parse_request_id(value: &str) -> Result<crate::share::DirectRequestId, String> {
    crate::share::DirectRequestId::parse(value.trim())
        .map_err(|_| format!("invalid direct request UUID: {value}"))
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
