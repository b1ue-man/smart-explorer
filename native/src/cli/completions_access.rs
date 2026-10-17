//! Completion of the access commands (PRM-M8, D.2.4): devices with their
//! state (`enabled · as se-remote · commands allow · terminal ask · until
//! 2026-10-17 18:00`), rooms, the values of `--as` and the ids of open
//! requests for `se share exec approve|deny`. Completion never starts the
//! background worker: open requests come from a running worker only.
use clap_complete::CompletionCandidate;

use crate::share::access_approval::ApprovalView;
use crate::share::access_grant::{Capability, REMOTE_ACCOUNT_NAME};
use crate::share::{ExecGrant, ShareProfiles};

/// The state of a grant as completion shows it.
pub(super) fn access_state_label(exec: &ExecGrant, now: i64) -> String {
    let access = exec.access_grant();
    let enabled = Capability::EXEC
        .into_iter()
        .any(|capability| access.decision_at(capability, now).is_on());
    let mut label = if enabled { "enabled" } else { "disabled" }.to_string();
    if let Some(account) = access.account.account_name() {
        label.push_str(&format!(" · as {account}"));
    }
    for capability in Capability::ALL {
        let decision = access.decision(capability);
        if capability.is_exec() || decision.is_on() {
            let name = capability_name(capability);
            label.push_str(&format!(" · {name} {}", decision.key()));
        }
    }
    if !exec.programs.is_empty() {
        label.push_str(&format!(" · programs {}", exec.programs.join(",")));
    }
    if let Some(end) = access.valid_until {
        let word = if access.expired_at(now) {
            "ended"
        } else {
            "until"
        };
        label.push_str(&format!(" · {word} {}", local_minutes(end)));
    }
    label
}

fn local_minutes(unix_secs: i64) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp(unix_secs, 0) {
        Some(time) => time
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => unix_secs.to_string(),
    }
}

fn profiles() -> Option<ShareProfiles> {
    ShareProfiles::load_checked(Some(super::share::default_home())).ok()
}

/// Devices of `se share grants access`: accepted direct grants and the
/// members of joined rooms that are not blocked.
pub(super) fn access_target_candidates() -> Vec<CompletionCandidate> {
    profiles()
        .map(|profiles| target_candidates(&profiles, crate::share::core_now_secs()))
        .unwrap_or_default()
}

pub(super) fn target_candidates(profiles: &ShareProfiles, now: i64) -> Vec<CompletionCandidate> {
    let accepted = crate::share::DirectGrantState::Accepted;
    let direct = profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == accepted)
        .map(|grant| {
            let value = format!("exec://direct/{}/{}", grant.device_id, grant.fingerprint);
            let state = access_state_label(&grant.exec, now);
            candidate(&value, format!("{} · direct · {state}", grant.device_name))
        });
    let rooms = profiles.rooms.iter().filter(|room| room.auto_join);
    let members = rooms.flat_map(|room| {
        room.members
            .iter()
            .filter(|member| !member.blocked)
            .map(move |member| {
                let value = format!(
                    "exec://room/{}/{}/{}",
                    room.room_id, member.device_id, member.fingerprint
                );
                let state = access_state_label(&member.exec, now);
                let help = format!("{} / {} · room · {state}", room.name, member.device_name);
                candidate(&value, help)
            })
    });
    direct.chain(members).collect()
}

/// Joined rooms for `--room`, by their local profile id.
pub(super) fn room_candidates() -> Vec<CompletionCandidate> {
    profiles()
        .map(|profiles| room_candidates_from(&profiles))
        .unwrap_or_default()
}

pub(super) fn room_candidates_from(profiles: &ShareProfiles) -> Vec<CompletionCandidate> {
    let joined = profiles.rooms.iter().filter(|room| room.auto_join);
    joined
        .map(|room| {
            let admitted = room.members.iter().filter(|m| m.is_admitted()).count();
            candidate(
                &room.id,
                format!("{} · {admitted} admitted members", room.name),
            )
        })
        .collect()
}

/// Values of `--as`: the own account, `se-remote` and every account a grant
/// of this host names already.
pub(super) fn account_candidates() -> Vec<CompletionCandidate> {
    account_candidates_from(&profiles().unwrap_or_default())
}

pub(super) fn account_candidates_from(profiles: &ShareProfiles) -> Vec<CompletionCandidate> {
    let mut values = vec![
        candidate("own", "the account of the Smart Explorer worker".into()),
        candidate(
            "remote",
            format!("the restricted account {REMOTE_ACCOUNT_NAME}"),
        ),
    ];
    let direct = profiles.direct_grants.iter().map(|grant| &grant.exec);
    let members = profiles.rooms.iter().flat_map(|room| room.members.iter());
    let mut named: Vec<String> = direct
        .chain(members.map(|member| &member.exec))
        .filter_map(|exec| {
            exec.access_grant()
                .account
                .account_name()
                .map(str::to_string)
        })
        .filter(|name| name != REMOTE_ACCOUNT_NAME)
        .collect();
    named.sort();
    named.dedup();
    values.extend(named.iter().map(|name| {
        candidate(
            &format!("account:{name}"),
            "a local account a grant names".into(),
        )
    }));
    values
}

/// Ids of the open requests for `approve` and `deny`; none without a
/// running worker.
pub(super) fn approval_candidates() -> Vec<CompletionCandidate> {
    crate::daemon::access_approvals()
        .map(|pending| approval_candidates_from(&pending))
        .unwrap_or_default()
}

pub(super) fn approval_candidates_from(pending: &[ApprovalView]) -> Vec<CompletionCandidate> {
    pending
        .iter()
        .map(|request| {
            let what = capability_name(request.capability);
            candidate(
                request.id.as_str(),
                format!("{} · {what}", request.device_name),
            )
        })
        .collect()
}

/// The capability as the English CLI names it.
pub(super) fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::ExecCommands => "commands",
        Capability::ExecTerminal => "terminal",
        Capability::ScreenView => "screen view",
        Capability::ScreenControl => "screen control",
    }
}

fn candidate(value: &str, help: String) -> CompletionCandidate {
    let help = help.replace(['\t', '\r', '\n'], " ");
    CompletionCandidate::new(value).help(Some(clap::builder::StyledStr::from(help)))
}

#[cfg(test)]
#[path = "completions_access_tests.rs"]
mod tests;
