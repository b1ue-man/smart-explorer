use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

#[derive(Args)]
pub(super) struct ExecGrantArgs {
    #[command(subcommand)]
    command: Option<ExecGrantCommand>,
}

#[derive(Subcommand)]
enum ExecGrantCommand {
    #[command(about = "Enable unrestricted user-code execution for exactly one device")]
    Enable(ExecToggleArgs),
    #[command(about = "Disable Exec and terminate that device's contained active commands")]
    Disable(ExecToggleArgs),
}

#[derive(Args)]
struct ExecToggleArgs {
    #[arg(
        help = "Optional device/name/fingerprint/endpoint selector; omit when exactly one choice matches",
        add = ArgValueCandidates::new(crate::cli::completions::exec_grant_candidates)
    )]
    selector: Option<String>,
    #[arg(
        long,
        help = "Confirm the FULL USER CODE EXECUTION warning without a prompt"
    )]
    yes: bool,
}

pub(super) fn run(args: ExecGrantArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let choices = choices(&profiles);
    let Some(command) = args.command else {
        return print_choices(&choices, json);
    };
    let (enabled, args) = match command {
        ExecGrantCommand::Enable(args) => (true, args),
        ExecGrantCommand::Disable(args) => (false, args),
    };
    let choice = select(&choices, args.selector.as_deref(), enabled)?;
    if enabled && !args.yes {
        confirm_enable(&choice.label)?;
    }
    let result = crate::daemon::mutate_exec_grant(choice.target.clone(), enabled)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "exec_grant\t{}\trequested={}\tpersisted={}\tapplied={}\trevision={}\tretry={:?}",
            clean(&choice.label),
            if enabled { "enabled" } else { "disabled" },
            result.persisted,
            result.applied,
            result.revision,
            result.retry_state,
        );
        if let Some(error) = &result.error {
            println!("error\t{}", clean(error));
        }
    }
    if result.persisted && result.applied {
        Ok(())
    } else {
        Err(format!(
            "Exec grant is fail-closed pending retry: persisted={}, applied={}, retry={:?}",
            result.persisted, result.applied, result.retry_state
        ))
    }
}

#[derive(Clone)]
struct ExecChoice {
    target: crate::share::ExecGrantTarget,
    label: String,
    device_id: String,
    fingerprint: String,
    enabled: bool,
}

fn choices(profiles: &crate::share::ShareProfiles) -> Vec<ExecChoice> {
    let mut choices = Vec::new();
    for grant in profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Accepted)
    {
        let label = profiles
            .direct_contacts
            .iter()
            .find(|contact| {
                contact.remote_device_id.as_ref() == Some(&grant.device_id)
                    && contact.remote_public_key.as_ref() == Some(&grant.public_key)
            })
            .map(|contact| contact.display_name.clone())
            .unwrap_or_else(|| grant.device_name.clone());
        choices.push(ExecChoice {
            target: crate::share::ExecGrantTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            label,
            device_id: grant.device_id.clone(),
            fingerprint: grant.fingerprint.clone(),
            enabled: grant.exec.enabled,
        });
    }
    for room in profiles.rooms.iter().filter(|room| room.auto_join) {
        for member in room.members.iter().filter(|member| !member.blocked) {
            choices.push(ExecChoice {
                target: crate::share::ExecGrantTarget::RoomMember {
                    room_id: room.room_id.clone(),
                    device_id: member.device_id.clone(),
                    public_key: member.public_key.clone(),
                    fingerprint: member.fingerprint.clone(),
                    node_id: member.node_id.clone(),
                },
                label: format!("{} / {}", room.name, member.device_name),
                device_id: member.device_id.clone(),
                fingerprint: member.fingerprint.clone(),
                enabled: member.exec.enabled,
            });
        }
    }
    choices
}

fn print_choices(choices: &[ExecChoice], json: bool) -> Result<(), String> {
    if json {
        let values = choices
            .iter()
            .map(|choice| {
                serde_json::json!({
                    "target": target_selector(&choice.target), "device_name": choice.label,
                    "device_id": choice.device_id, "fingerprint": choice.fingerprint,
                    "enabled": choice.enabled,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&values).map_err(|e| e.to_string())?
        );
    } else if choices.is_empty() {
        println!("exec_grants\t0\taccept a direct request or join a room first");
    } else {
        for choice in choices {
            println!(
                "exec_grant\t{}\tdevice_id={}\tfingerprint={}\tstate={}\ttarget={}",
                clean(&choice.label),
                choice.device_id,
                choice.fingerprint,
                if choice.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                target_selector(&choice.target),
            );
        }
    }
    Ok(())
}

fn select<'a>(
    choices: &'a [ExecChoice],
    selector: Option<&str>,
    enabling: bool,
) -> Result<&'a ExecChoice, String> {
    let matches = choices
        .iter()
        .filter(|choice| choice.enabled != enabling)
        .filter(|choice| {
            selector.is_none_or(|selector| {
                choice.label.eq_ignore_ascii_case(selector)
                    || prefix(selector, &choice.device_id)
                    || prefix(selector, &choice.fingerprint)
                    || target_selector(&choice.target) == selector
            })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [choice] => Ok(*choice),
        [] => Err("no matching Exec grant; run `se share grants exec` to list choices".into()),
        _ => Err(format!(
            "multiple Exec grants match; choose one: {}",
            matches
                .iter()
                .map(|choice| target_selector(&choice.target))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn confirm_enable(label: &str) -> Result<(), String> {
    use std::io::Write;
    let provider = crate::share::exec_provider_status();
    eprintln!(
        "WARNING: enabling FULL {} CODE EXECUTION for {label}.",
        provider.user_label
    );
    if provider.elevated {
        eprintln!(
            "This is a REMOTE {} SHELL with unrestricted authority.",
            if cfg!(windows) {
                "ADMINISTRATOR"
            } else {
                "ROOT"
            }
        );
    }
    eprint!("Type YES to enable: ");
    std::io::stderr().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    (answer.trim() == "YES")
        .then_some(())
        .ok_or_else(|| "Exec grant was not enabled".into())
}

fn prefix(selector: &str, candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case(selector)
        || (selector.len() >= 4
            && candidate
                .to_ascii_lowercase()
                .starts_with(&selector.to_ascii_lowercase()))
}

fn target_selector(target: &crate::share::ExecGrantTarget) -> String {
    match target {
        crate::share::ExecGrantTarget::Direct {
            device_id,
            fingerprint,
            ..
        } => format!("exec://direct/{device_id}/{fingerprint}"),
        crate::share::ExecGrantTarget::RoomMember {
            room_id,
            device_id,
            fingerprint,
            ..
        } => format!("exec://room/{room_id}/{device_id}/{fingerprint}"),
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_free_toggle_considers_only_grants_that_need_the_change() {
        let choices = vec![choice("enabled", true), choice("disabled", false)];
        assert_eq!(select(&choices, None, true).unwrap().label, "disabled");
        assert_eq!(select(&choices, None, false).unwrap().label, "enabled");
    }

    fn choice(label: &str, enabled: bool) -> ExecChoice {
        ExecChoice {
            target: crate::share::ExecGrantTarget::Direct {
                device_id: label.into(),
                public_key: format!("{label}-key"),
                fingerprint: format!("{label}-fingerprint"),
                node_id: format!("{label}-node"),
            },
            label: label.into(),
            device_id: label.into(),
            fingerprint: format!("{label}-fingerprint"),
            enabled,
        }
    }
}
