use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

#[derive(Args)]
pub(super) struct ExecGrantArgs {
    #[command(subcommand)]
    command: Option<ExecGrantCommand>,
}

#[derive(Subcommand)]
enum ExecGrantCommand {
    #[command(
        about = "Enable user-code execution for exactly one device",
        long_about = ENABLE_HELP
    )]
    Enable(ExecEnableArgs),
    #[command(about = "Disable Exec and terminate that device's contained active commands")]
    Disable(ExecToggleArgs),
}

const ENABLE_HELP: &str = "\
Enable user-code execution for exactly one device.

Without options the device gets the full shell of the Standardnutzer: the
account this Smart Explorer worker runs as, exactly as every grant made before
these options existed. On a Linux host whose worker runs as root,
--run-as USER runs the device's commands as that local account instead.
--allow-program PATH (repeatable) limits the device to starting exactly these
programs in argv mode: no shell command and no terminal. Changing an already
enabled grant's account or programs ends that device's running commands.

A program is a bare name or an absolute path. It is found only by that
absolute path or in the host's fixed system search path, never through a PATH
the device sends, and it starts only if it is the same file (under the same
name) as a listed program. The device may pass only LANG, LC_* and TERM as
environment variables; every other variable (for example PATH, LD_PRELOAD or
GIT_PAGER) refuses the start. Arguments stay free, so shells, env,
interpreters (for example python, perl, node) and programs that run commands
(for example find, git, ssh, xargs, awk, sudo) are equal to a full shell: do
not list them.";

#[derive(Args, Clone)]
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

#[derive(Args)]
struct ExecEnableArgs {
    #[command(flatten)]
    toggle: ExecToggleArgs,
    #[arg(
        long,
        value_name = "USER",
        help = "Run this device's commands as USER (Linux root worker); `standard` is the worker's own account"
    )]
    run_as: Option<String>,
    #[arg(
        long = "allow-program",
        value_name = "PATH",
        help = "Allow only this program (repeatable): argv mode, no shell, no terminal; free arguments, so no shells, interpreters or command runners"
    )]
    allow_program: Vec<String>,
    #[arg(
        long,
        conflicts_with = "allow_program",
        help = "Remove the program list again: full shell"
    )]
    full: bool,
}

impl ExecEnableArgs {
    /// `Some` when an option names new settings; the rest is taken from
    /// the grant's current ones.
    fn settings(
        &self,
        current: &crate::share::ExecGrantSettings,
    ) -> Option<crate::share::ExecGrantSettings> {
        if self.run_as.is_none() && self.allow_program.is_empty() && !self.full {
            return None;
        }
        let run_as = match self.run_as.as_deref() {
            None => current.run_as.clone(),
            Some("standard") | Some("") => crate::share::ExecRunAs::Standard,
            Some(name) => crate::share::ExecRunAs::Account { name: name.into() },
        };
        let programs = if self.full {
            Vec::new()
        } else if self.allow_program.is_empty() {
            current.programs.clone()
        } else {
            self.allow_program.clone()
        };
        Some(crate::share::ExecGrantSettings { run_as, programs })
    }
}

pub(super) fn run(args: ExecGrantArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let choices = choices(&profiles);
    let Some(command) = args.command else {
        return print_choices(&choices, json);
    };
    let (enabled, args, enable) = match command {
        ExecGrantCommand::Enable(args) => (true, args.toggle.clone(), Some(args)),
        ExecGrantCommand::Disable(args) => (false, args, None),
    };
    // A settings change may address an already enabled grant.
    let changes_settings = enable.as_ref().is_some_and(|enable| {
        enable.run_as.is_some() || !enable.allow_program.is_empty() || enable.full
    });
    let choice = select(
        &choices,
        args.selector.as_deref(),
        enabled,
        changes_settings,
    )?;
    let settings = enable
        .as_ref()
        .and_then(|enable| enable.settings(&choice.settings));
    if let Some(settings) = &settings {
        settings.validate_for_this_host()?;
    }
    if enabled && !args.yes {
        confirm_enable(&choice.label, settings.as_ref().unwrap_or(&choice.settings))?;
    }
    let result =
        crate::daemon::mutate_exec_grant(choice.target.clone(), enabled, settings.clone())?;
    if let Some(requested) = &settings {
        verify_settings(&choice.target, requested)?;
    }
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
    settings: crate::share::ExecGrantSettings,
}

fn grant_settings(grant: &crate::share::ExecGrant) -> crate::share::ExecGrantSettings {
    crate::share::ExecGrantSettings {
        run_as: grant.run_as.clone(),
        programs: grant.programs.clone(),
    }
}

/// The worker that applied the change wrote the requested settings; a worker
/// from before run-as would have ignored them and must not look successful.
fn verify_settings(
    target: &crate::share::ExecGrantTarget,
    requested: &crate::share::ExecGrantSettings,
) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let stored = choices(&profiles)
        .into_iter()
        .find(|choice| &choice.target == target)
        .map(|choice| choice.settings);
    if stored.as_ref() == Some(requested) {
        Ok(())
    } else {
        Err(
            "the background worker did not store the requested account/program settings \
             (a worker older than this `se` ignores them); finish the update so the worker \
             restarts, then try again"
                .into(),
        )
    }
}

fn settings_text(settings: &crate::share::ExecGrantSettings, worker: &str) -> String {
    let account = match settings.run_as.account_name() {
        None => format!("standard({worker})"),
        Some(name) => name.to_string(),
    };
    if settings.programs.is_empty() {
        format!("run_as={account}\tprograms=full-shell")
    } else {
        let programs: Vec<String> = settings
            .programs
            .iter()
            .map(|program| clean(program))
            .collect();
        format!("run_as={account}\tprograms={}", programs.join(","))
    }
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
            settings: grant_settings(&grant.exec),
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
                settings: grant_settings(&member.exec),
            });
        }
    }
    choices
}

fn print_choices(choices: &[ExecChoice], json: bool) -> Result<(), String> {
    let worker = crate::share::exec_provider_status().user_label;
    if json {
        let values = choices
            .iter()
            .map(|choice| {
                serde_json::json!({
                    "target": target_selector(&choice.target), "device_name": choice.label,
                    "device_id": choice.device_id, "fingerprint": choice.fingerprint,
                    "enabled": choice.enabled,
                    "run_as": choice.settings.run_as.account_name().unwrap_or("standard"),
                    "standard_user": worker,
                    "programs": choice.settings.programs,
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
                "exec_grant\t{}\tdevice_id={}\tfingerprint={}\tstate={}\t{}\ttarget={}",
                clean(&choice.label),
                choice.device_id,
                choice.fingerprint,
                if choice.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                settings_text(&choice.settings, &worker),
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
    changes_settings: bool,
) -> Result<&'a ExecChoice, String> {
    let matches = choices
        .iter()
        .filter(|choice| choice.enabled != enabling || changes_settings)
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

fn confirm_enable(label: &str, settings: &crate::share::ExecGrantSettings) -> Result<(), String> {
    use std::io::Write;
    let provider = crate::share::exec_provider_status();
    let account = settings
        .run_as
        .account_name()
        .map(str::to_string)
        .unwrap_or_else(|| provider.user_label.clone());
    if settings.programs.is_empty() {
        eprintln!("WARNING: enabling FULL {account} CODE EXECUTION for {label}.");
    } else {
        eprintln!(
            "WARNING: enabling {account} execution of {} for {label}.",
            settings.programs.join(", ")
        );
    }
    if provider.elevated && settings.run_as.account_name().is_none() {
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
        assert_eq!(
            select(&choices, None, true, false).unwrap().label,
            "disabled"
        );
        assert_eq!(
            select(&choices, None, false, false).unwrap().label,
            "enabled"
        );
        assert!(select(&choices, None, true, true).is_err());
        assert_eq!(
            select(&choices, Some("enabled"), true, true).unwrap().label,
            "enabled"
        );
    }

    #[test]
    fn luecken_task_enable_options_build_settings_from_the_current_grant() {
        let current = crate::share::ExecGrantSettings {
            run_as: crate::share::ExecRunAs::Account {
                name: "deploy".into(),
            },
            programs: vec!["/usr/bin/uptime".into()],
        };
        let args = |run_as: Option<&str>, programs: &[&str], full: bool| ExecEnableArgs {
            toggle: ExecToggleArgs {
                selector: None,
                yes: true,
            },
            run_as: run_as.map(str::to_string),
            allow_program: programs.iter().map(|program| program.to_string()).collect(),
            full,
        };
        assert_eq!(args(None, &[], false).settings(&current), None);
        let standard = args(Some("standard"), &[], false)
            .settings(&current)
            .unwrap();
        assert_eq!(standard.run_as, crate::share::ExecRunAs::Standard);
        assert_eq!(standard.programs, current.programs);
        let full = args(None, &[], true).settings(&current).unwrap();
        assert_eq!(full.run_as, current.run_as);
        assert!(full.programs.is_empty());
        let listed = args(None, &["/bin/date"], false)
            .settings(&current)
            .unwrap();
        assert_eq!(listed.programs, vec!["/bin/date".to_string()]);
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
            settings: Default::default(),
        }
    }
}
