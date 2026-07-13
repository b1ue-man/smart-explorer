use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use super::lifecycle_output;
use super::request_selection::{
    ambiguous_pending_error, is_pending_incoming, legacy_accept_eligible, legacy_retryable,
    legacy_selector_matches, matching_legacy, matching_tracked, no_acceptable_error,
    no_pending_error, pending_legacy, tracked_accept_eligible, tracked_deletable,
    tracked_retryable, verify_optional_fingerprint,
};
use super::requests_support::{worker_refresh, WorkerRefresh};

#[derive(Args)]
#[command(long_about = "Inspect and decide durable direct access requests.\n\n\
With no subcommand, this shows the pending incoming inbox and the exact next\n\
command. `accept` needs no selector when exactly one non-conflicting request is\n\
accept-eligible; `reject` needs none when exactly one request is pending.\n\
`show`, `retry`, and `delete` likewise auto-select their sole eligible entry.\n\
Selectors can be copied from this command's output or completed with the shell\n\
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
    #[command(about = "Accept an eligible incoming request; conflicts are excluded")]
    Accept(AcceptDecisionArgs),
    #[command(about = "Reject a pending incoming request; auto-selects the only one")]
    Reject(DecisionArgs),
    #[command(about = "Retry a tracked envelope or an unconfirmed legacy decision now")]
    Retry(RetrySelectionArgs),
    #[command(about = "Delete a request locally, stop retries, and retain a replay tombstone")]
    Delete(DeleteSelectionArgs),
}

#[derive(Args)]
struct AcceptDecisionArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional accept-eligible request selector; conflicts are excluded",
        add = ArgValueCandidates::new(crate::cli::completions::acceptable_request_candidates)
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

impl From<AcceptDecisionArgs> for DecisionArgs {
    fn from(value: AcceptDecisionArgs) -> Self {
        Self {
            selector: value.selector,
            fingerprint: value.fingerprint,
            message: value.message,
        }
    }
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
struct RetrySelectionArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional request selector shown by request list; omit when only one is eligible",
        add = ArgValueCandidates::new(crate::cli::completions::retry_request_candidates)
    )]
    selector: Option<String>,
}

#[derive(Args)]
struct DeleteSelectionArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Optional deletable request selector shown by request list; omit when only one is eligible",
        add = ArgValueCandidates::new(crate::cli::completions::deletable_request_candidates)
    )]
    selector: Option<String>,
}

pub(super) fn run(args: RequestArgs) -> Result<(), String> {
    match args.command {
        None => super::requests_inbox::run(args.json),
        Some(RequestCommand::List) => list(args.json),
        Some(RequestCommand::Show(command)) => show(command.selector.as_deref(), args.json),
        Some(RequestCommand::Accept(command)) => decide(
            command.into(),
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

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    if json {
        let requests = profiles
            .direct_requests
            .iter()
            .map(|entry| lifecycle_output::request_value(entry, &profiles))
            .collect::<Vec<_>>();
        let legacy_requests = profiles
            .legacy_direct_requests
            .iter()
            .map(|entry| super::requests_legacy::value(entry, &profiles))
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": requests.len() + legacy_requests.len(),
                "requests": requests,
                "legacy_requests": legacy_requests,
            }))
            .map_err(|error| error.to_string())?
        );
    } else if profiles.direct_requests.is_empty() && profiles.legacy_direct_requests.is_empty() {
        println!("requests\t0");
    } else {
        for entry in &profiles.direct_requests {
            for line in lifecycle_output::request_text(entry, &profiles) {
                println!("{line}");
            }
        }
        for entry in &profiles.legacy_direct_requests {
            println!("{}", super::requests_legacy::text(entry, &profiles));
        }
    }
    Ok(())
}

fn show(selector: Option<&str>, json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let tracked = matching_tracked(&profiles, selector, |_| true);
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| selector.is_none_or(|value| legacy_selector_matches(entry, value.trim())))
        .collect::<Vec<_>>();
    match tracked.len() + legacy.len() {
        1 if tracked.len() == 1 => {
            lifecycle_output::print_request(tracked[0], &profiles, json);
            Ok(())
        }
        1 => super::requests_legacy::print(legacy[0], &profiles, json),
        0 => Err(format!(
            "request not found: {}",
            selector.unwrap_or("<only item>")
        )),
        _ => Err("request selector is ambiguous; run `se share request list`".into()),
    }
}

fn decide(
    args: DecisionArgs,
    decision: crate::share::DirectDecisionKind,
    json: bool,
) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let tracked_matches = matching_tracked(&profiles, args.selector.as_deref(), |entry| {
        if decision == crate::share::DirectDecisionKind::Accepted {
            tracked_accept_eligible(&profiles, entry, now)
        } else {
            is_pending_incoming(entry, now)
        }
    });
    let legacy = pending_legacy(&profiles, now);
    let eligible_legacy = legacy
        .iter()
        .copied()
        .filter(|entry| {
            decision != crate::share::DirectDecisionKind::Accepted
                || legacy_accept_eligible(entry, now)
        })
        .collect::<Vec<_>>();
    let legacy_matches = matching_legacy(&eligible_legacy, args.selector.as_deref());
    match tracked_matches.len() + legacy_matches.len() {
        0 => {
            return Err(if decision == crate::share::DirectDecisionKind::Accepted {
                no_acceptable_error(args.selector.as_deref(), &profiles, now)
            } else {
                no_pending_error(args.selector.as_deref(), &profiles, &legacy, now)
            })
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
    let tracked = matching_tracked(&profiles, selector, |entry| tracked_retryable(entry, now));
    let legacy = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| legacy_retryable(entry))
        .filter(|entry| selector.is_none_or(|value| legacy_selector_matches(entry, value.trim())))
        .collect::<Vec<_>>();
    if tracked.len() + legacy.len() != 1 {
        return Err(if tracked.is_empty() && legacy.is_empty() {
            format!(
                "retryable request not found: {}",
                selector.unwrap_or("<only item>")
            )
        } else {
            "retryable request selector is ambiguous; run `se share request list`".into()
        });
    }
    if tracked.is_empty() {
        let selector = legacy[0].selector.clone();
        let persisted =
            crate::share::retry_legacy_direct_answer(Some(super::default_home()), &selector)?;
        let worker = worker_refresh();
        let committed = super::checked_profiles()?;
        let entry = committed
            .legacy_direct_request(&selector)
            .cloned()
            .unwrap_or(persisted);
        return super::requests_legacy::print_action(
            &entry,
            &committed,
            "retry_queued",
            worker,
            json,
        );
    }
    let request_id = tracked[0].record.request.request_id.clone();
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
    let now = crate::share::core_now_secs();
    let tracked = matching_tracked(&profiles, selector, |entry| tracked_deletable(entry, now));
    let legacy_all = profiles
        .legacy_direct_requests
        .iter()
        .filter(|entry| selector.is_none_or(|value| legacy_selector_matches(entry, value.trim())))
        .collect::<Vec<_>>();
    let legacy = legacy_all
        .iter()
        .copied()
        .filter(|entry| !entry.authorization_active(&profiles))
        .collect::<Vec<_>>();
    if tracked.len() + legacy.len() != 1 {
        if tracked.is_empty()
            && legacy.is_empty()
            && legacy_all.len() == 1
            && legacy_all[0].authorization_active(&profiles)
        {
            return Err(format!(
                "legacy request has an active authorization; run `se share grants revoke {}` before deletion",
                legacy_all[0].selector
            ));
        }
        return Err(if tracked.is_empty() && legacy.is_empty() {
            format!(
                "deletable request not found: {}",
                selector.unwrap_or("<only item>")
            )
        } else {
            "deletable request selector is ambiguous; run `se share request list`".into()
        });
    }
    if tracked.is_empty() {
        let legacy_selector = legacy[0].selector.clone();
        crate::share::delete_legacy_direct_request(Some(super::default_home()), &legacy_selector)?;
        let worker = worker_refresh();
        return super::requests_legacy::print_deleted(
            "selector",
            &legacy_selector,
            true,
            worker,
            json,
        );
    }
    let request_id = tracked[0].record.request.request_id.clone();
    crate::share::delete_direct_request_history(Some(super::default_home()), &request_id)?;
    let worker = worker_refresh();
    super::requests_legacy::print_deleted("request_id", request_id.as_str(), false, worker, json)
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
    request: crate::share::LegacyDirectRequestEntry,
    accepted: bool,
    json: bool,
) -> Result<(), String> {
    if args.message.is_some() {
        return Err(
            "legacy requests do not support authenticated decision messages; omit --message".into(),
        );
    }
    verify_optional_fingerprint(
        args.fingerprint.as_deref(),
        &request.peer.fingerprint,
        &request.selector,
    )?;
    let identity = super::identity_command::load_with_repair_hint()?;
    let persisted = crate::share::decide_legacy_direct_request(
        Some(super::default_home()),
        &identity,
        &request.selector,
        &request.peer.fingerprint,
        accepted,
    )?;
    let worker = worker_refresh();
    let committed = super::checked_profiles()?;
    let entry = committed
        .legacy_direct_request(&request.selector)
        .cloned()
        .unwrap_or(persisted);
    super::requests_legacy::print_action(
        &entry,
        &committed,
        if accepted { "accepted" } else { "rejected" },
        worker,
        json,
    )
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod tests;
