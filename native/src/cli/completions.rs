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
}
