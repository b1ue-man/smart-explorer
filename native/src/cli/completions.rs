use clap::{Args, ValueEnum};
use clap_complete::env::{Bash, Elvish, EnvCompleter, Fish, Powershell, Zsh};
use std::io;

#[derive(Args)]
#[command(long_about = "Generate dynamic shell completion setup for se.\n\n\
The generated integration completes commands, options, and live Share request,\n\
grant, and peer selectors. Source it from your shell profile, for example:\n\
  Bash:       source <(se completions bash)\n\
  Zsh:        source <(se completions zsh)\n\
  Fish:       se completions fish | source\n\
  PowerShell: se completions powershell | Out-String | Invoke-Expression")]
pub(super) struct CompletionsArgs {
    #[arg(value_enum, help = "Shell to generate completion setup for")]
    shell: CompletionShell,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

pub(super) fn run(args: CompletionsArgs) -> Result<i32, String> {
    let completer: &dyn EnvCompleter = match args.shell {
        CompletionShell::Bash => &Bash,
        CompletionShell::Elvish => &Elvish,
        CompletionShell::Fish => &Fish,
        CompletionShell::Powershell => &Powershell,
        CompletionShell::Zsh => &Zsh,
    };
    completer
        .write_registration("COMPLETE", "se", "se", "se", &mut io::stdout())
        .map_err(|error| format!("generate {} completions: {error}", completer.name()))?;
    Ok(0)
}

pub(super) fn request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    profiles
        .direct_requests
        .iter()
        .map(|entry| {
            let request = &entry.record.request;
            let peer = match entry.direction {
                crate::share::DirectRequestDirection::Incoming => &request.requester,
                crate::share::DirectRequestDirection::Outgoing => &request.target,
            };
            candidate(
                request.request_id.as_str(),
                format!(
                    "{} · {} · decision={} · delivery={}",
                    direction(entry.direction),
                    peer.device_name,
                    entry.record.decision.state.code(),
                    entry.record.delivery.state.code(),
                ),
            )
        })
        .collect()
}

pub(super) fn pending_request_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let now = crate::share::core_now_secs();
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    profiles
        .direct_requests
        .iter()
        .filter(|entry| {
            entry.direction == crate::share::DirectRequestDirection::Incoming
                && entry.record.decision.state == crate::share::DirectDecisionState::Pending
                && entry.record.request.expires_at >= now
        })
        .map(|entry| {
            let request = &entry.record.request;
            candidate(
                request.request_id.as_str(),
                format!(
                    "{} · fingerprint {}",
                    request.requester.device_name, request.requester.fingerprint
                ),
            )
        })
        .collect()
}

pub(super) fn active_grant_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Accepted)
        .map(|grant| {
            candidate(
                &grant.device_id,
                format!(
                    "{} · fingerprint {} · active",
                    grant.device_name, grant.fingerprint
                ),
            )
        })
        .collect()
}

pub(super) fn exec_grant_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    exec_grant_candidates_from_profiles(&profiles)
}

fn exec_grant_candidates_from_profiles(
    profiles: &crate::share::ShareProfiles,
) -> Vec<clap_complete::CompletionCandidate> {
    let direct = profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Accepted)
        .map(|grant| {
            candidate(
                &format!("exec://direct/{}/{}", grant.device_id, grant.fingerprint),
                format!(
                    "{} · direct · {}",
                    grant.device_name,
                    if grant.exec.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            )
        });
    let rooms = profiles
        .rooms
        .iter()
        .filter(|room| room.auto_join)
        .flat_map(|room| {
            room.members
                .iter()
                .filter(|member| !member.blocked)
                .map(|member| {
                    candidate(
                        &format!(
                            "exec://room/{}/{}/{}",
                            room.room_id, member.device_id, member.fingerprint
                        ),
                        format!(
                            "{} / {} · room · {}",
                            room.name,
                            member.device_name,
                            if member.exec.enabled {
                                "enabled"
                            } else {
                                "disabled"
                            }
                        ),
                    )
                })
        });
    direct.chain(rooms).collect()
}

pub(super) fn peer_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    profiles
        .direct_contacts
        .iter()
        .map(|contact| {
            candidate(
                &contact.id,
                format!(
                    "{} · fingerprint {} · {}",
                    contact.display_name,
                    contact.expected_fingerprint,
                    contact.access_state.label()
                ),
            )
        })
        .collect()
}

pub(super) fn exec_target_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(profiles) =
        crate::share::ShareProfiles::load_checked(Some(super::share::default_home()))
    else {
        return Vec::new();
    };
    let mut values = profiles
        .direct_contacts
        .iter()
        .map(|contact| {
            candidate(
                &format!("share://direct/{}", contact.id),
                format!(
                    "{} · {} · device {}",
                    contact.display_name,
                    contact.status.label(),
                    contact.remote_device_id.as_deref().unwrap_or("unknown")
                ),
            )
        })
        .collect::<Vec<_>>();
    values.extend(profiles.rooms.iter().flat_map(|room| {
        room.members.iter().map(|member| {
            candidate(
                &format!("share://room/{}/{}", room.room_id, member.device_id),
                format!(
                    "{} / {} · {}",
                    room.name,
                    member.device_name,
                    member.status.label()
                ),
            )
        })
    }));
    values
}

pub(super) fn exec_id_candidates() -> Vec<clap_complete::CompletionCandidate> {
    let Ok(snapshot) = crate::daemon::exec_jobs() else {
        return Vec::new();
    };
    exec_id_candidates_from_snapshot(&snapshot)
}

fn exec_id_candidates_from_snapshot(
    snapshot: &crate::daemon::ExecJobsSnapshot,
) -> Vec<clap_complete::CompletionCandidate> {
    snapshot
        .outgoing_active
        .iter()
        .map(|job| (crate::daemon::ExecJobDirection::Outgoing, job))
        .chain(
            snapshot
                .incoming_active
                .iter()
                .map(|job| (crate::daemon::ExecJobDirection::Incoming, job)),
        )
        .chain(
            snapshot
                .outgoing_history
                .iter()
                .map(|job| (crate::daemon::ExecJobDirection::Outgoing, job)),
        )
        .chain(
            snapshot
                .incoming_history
                .iter()
                .map(|job| (crate::daemon::ExecJobDirection::Incoming, job)),
        )
        .map(|(direction, job)| {
            candidate(
                &exec_job_selector(direction, &job.exec_id),
                format!(
                    "{} · {} · {:?}",
                    job.peer_device_name, job.program, job.state
                ),
            )
        })
        .collect()
}

pub(super) fn exec_job_selector(
    direction: crate::daemon::ExecJobDirection,
    id: &crate::share::ExecId,
) -> String {
    let direction = match direction {
        crate::daemon::ExecJobDirection::Outgoing => "outgoing",
        crate::daemon::ExecJobDirection::Incoming => "incoming",
    };
    format!("{direction}:{id}")
}

fn candidate(value: &str, help: String) -> clap_complete::CompletionCandidate {
    let help = help.replace(['\t', '\r', '\n'], " ");
    clap_complete::CompletionCandidate::new(value).help(Some(clap::builder::StyledStr::from(help)))
}

fn direction(value: crate::share::DirectRequestDirection) -> &'static str {
    match value {
        crate::share::DirectRequestDirection::Incoming => "incoming",
        crate::share::DirectRequestDirection::Outgoing => "outgoing",
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::super::Cli;

    #[test]
    fn parses_every_supported_completion_shell() {
        for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
            assert!(Cli::try_parse_from(["se", "completions", shell]).is_ok());
        }
    }

    #[test]
    fn dynamic_engine_completes_nested_share_commands() {
        let candidates = clap_complete::engine::complete(
            &mut Cli::command(),
            ["se", "share", "req"]
                .map(std::ffi::OsString::from)
                .to_vec(),
            2,
            None,
        )
        .unwrap();
        assert!(candidates
            .iter()
            .any(|candidate| candidate.get_value() == "request"));
    }

    #[test]
    fn dynamic_engine_completes_exec_management_without_prior_ids() {
        for (words, index, expected) in [
            (vec!["se", "ex"], 1, "exec"),
            (vec!["se", "share", "gr"], 2, "grants"),
            (vec!["se", "share", "grants", "ex"], 3, "exec"),
            (vec!["se", "share", "exec", "ca"], 3, "cancel"),
        ] {
            let candidates = clap_complete::engine::complete(
                &mut Cli::command(),
                words.into_iter().map(std::ffi::OsString::from).collect(),
                index,
                None,
            )
            .unwrap();
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate.get_value() == expected),
                "missing completion {expected}: {candidates:?}"
            );
        }
    }

    #[test]
    fn colliding_exec_ids_have_distinct_directional_completion_values() {
        let id = crate::share::ExecId::parse("44".repeat(16)).unwrap();
        let snapshot = crate::daemon::ExecJobsSnapshot {
            outgoing_active: vec![job(id.clone(), "out-peer")],
            incoming_active: vec![job(id.clone(), "in-peer")],
            ..Default::default()
        };
        let values = super::exec_id_candidates_from_snapshot(&snapshot)
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(values.contains(&format!("outgoing:{id}")));
        assert!(values.contains(&format!("incoming:{id}")));
    }

    #[test]
    fn exec_grant_completion_covers_exact_direct_and_room_targets() {
        let mut profiles = crate::share::ShareProfiles::default();
        profiles.direct_grants.push(crate::share::DirectGrant {
            device_id: "direct-device".into(),
            device_name: "Direct".into(),
            public_key: "direct-key".into(),
            fingerprint: "direct-fp".into(),
            node_id: "direct-key".into(),
            state: crate::share::DirectGrantState::Accepted,
            updated_at: 1,
            exec: Default::default(),
        });
        profiles.rooms.push(crate::share::RoomProfile {
            id: "local-room-profile".into(),
            name: "Room".into(),
            room_id: "wire-room".into(),
            auto_join: true,
            last_seen: None,
            status: crate::share::ShareStatus::Waiting,
            members: vec![crate::share::RoomMember {
                device_id: "room-device".into(),
                device_name: "Member".into(),
                fingerprint: "room-fp".into(),
                public_key: "room-key".into(),
                node_id: "room-key".into(),
                relay_url: String::new(),
                candidates: Vec::new(),
                last_seen: None,
                status: crate::share::ShareStatus::Waiting,
                blocked: false,
                exec: Default::default(),
                presence: None,
            }],
            exports: Default::default(),
        });
        let values = super::exec_grant_candidates_from_profiles(&profiles)
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(values
            .iter()
            .any(|value| value == "exec://direct/direct-device/direct-fp"));
        assert!(values
            .iter()
            .any(|value| value == "exec://room/wire-room/room-device/room-fp"));
        assert!(values
            .iter()
            .all(|value| !value.contains("local-room-profile")));
    }

    fn job(id: crate::share::ExecId, peer: &str) -> crate::share::ExecJobView {
        crate::share::ExecJobView {
            exec_id: id,
            peer_device_id: peer.into(),
            peer_device_name: peer.into(),
            program: "<shell>".into(),
            command_digest: "digest".into(),
            state: crate::share::ExecLifecycleState::Running,
            policy_revision: 1,
            started_at: Some(1),
            finished_at: None,
            terminal: None,
        }
    }
}
