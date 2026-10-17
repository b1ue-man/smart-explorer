//! `se share grants exec` and its toggles `enable`/`disable` (aliases of the
//! access right since PRM-M8: they keep every option, the IPC request
//! `MutateExecGrant` and their output), and the device selection that `se
//! share grants access` shares with them: accepted direct grants and members
//! of joined rooms that are not blocked, one device by selector or every
//! admitted member of a room.
use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::share::{ExecGrant, ExecGrantTarget, RoomMember, RoomProfile, ShareProfiles};

use super::grants_access_output::{self as output, confirm_enable, RoomFacts};

#[derive(Args)]
pub(super) struct ExecGrantArgs {
    #[command(subcommand)]
    pub(super) command: Option<ExecGrantCommand>,
}

#[derive(Subcommand)]
pub(super) enum ExecGrantCommand {
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
not list them.

`se share grants access` sets the same right with more choices: commands and
terminal separately, a confirmation before each start, the account se-remote
and an end.";

#[derive(Args, Clone)]
pub(super) struct ExecToggleArgs {
    #[arg(
        help = "Optional device/name/fingerprint/endpoint selector; omit when exactly one choice matches",
        add = ArgValueCandidates::new(crate::cli::completions::exec_grant_candidates)
    )]
    pub(super) selector: Option<String>,
    #[arg(long, help = "Confirm the access warning without a prompt")]
    pub(super) yes: bool,
}

#[derive(Args)]
pub(super) struct ExecEnableArgs {
    #[command(flatten)]
    pub(super) toggle: ExecToggleArgs,
    #[arg(
        long,
        value_name = "USER",
        help = "Run this device's commands as USER (Linux root worker); `standard` is the worker's own account"
    )]
    pub(super) run_as: Option<String>,
    #[arg(
        long = "allow-program",
        value_name = "PATH",
        help = "Allow only this program (repeatable): argv mode, no shell, no terminal; free arguments, so no shells, interpreters or command runners"
    )]
    pub(super) allow_program: Vec<String>,
    #[arg(
        long,
        conflicts_with = "allow_program",
        help = "Remove the program list again: full shell"
    )]
    pub(super) full: bool,
}

impl ExecEnableArgs {
    fn names_settings(&self) -> bool {
        self.run_as.is_some() || !self.allow_program.is_empty() || self.full
    }

    /// `Some` when an option names new settings; the rest is taken from
    /// the grant's current ones.
    pub(super) fn settings(
        &self,
        current: &crate::share::ExecGrantSettings,
    ) -> Option<crate::share::ExecGrantSettings> {
        if !self.names_settings() {
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

/// The IPC request of a toggle (`MutateExecGrant`): enabled and the
/// settings, unchanged by the access right (D.2.3).
pub(super) fn toggle_request(
    command: &ExecGrantCommand,
    current: &ExecGrant,
) -> (bool, Option<crate::share::ExecGrantSettings>) {
    match command {
        ExecGrantCommand::Enable(args) => (true, args.settings(&grant_settings(current))),
        ExecGrantCommand::Disable(_) => (false, None),
    }
}

pub(super) fn run(args: ExecGrantArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let choices = targets(&profiles);
    let Some(command) = args.command else {
        return output::print_exec_choices(&choices, json);
    };
    let (toggle, changes_settings) = match &command {
        ExecGrantCommand::Enable(args) => (args.toggle.clone(), args.names_settings()),
        ExecGrantCommand::Disable(args) => (args.clone(), false),
    };
    let enabling = matches!(command, ExecGrantCommand::Enable(_));
    let choice = select(
        &choices,
        toggle.selector.as_deref(),
        enabling,
        changes_settings,
    )?;
    let (enabled, settings) = toggle_request(&command, &choice.exec);
    if let Some(settings) = &settings {
        settings.validate_for_this_host()?;
    }
    if enabled && !toggle.yes {
        let current = grant_settings(&choice.exec);
        confirm_enable(&choice.label, settings.as_ref().unwrap_or(&current))?;
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

pub(in crate::cli::share) fn grant_settings(grant: &ExecGrant) -> crate::share::ExecGrantSettings {
    crate::share::ExecGrantSettings {
        run_as: grant.run_as.clone(),
        programs: grant.programs.clone(),
    }
}

/// The worker that applied the change wrote the requested settings; a worker
/// from before run-as would have ignored them and must not look successful.
fn verify_settings(
    target: &ExecGrantTarget,
    requested: &crate::share::ExecGrantSettings,
) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let stored = targets(&profiles)
        .into_iter()
        .find(|choice| &choice.target == target)
        .map(|choice| grant_settings(&choice.exec));
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

/// One device whose rights this host sets: an accepted direct grant or a
/// member of a joined room that is not blocked.
#[derive(Clone, Debug)]
pub(super) struct AccessTarget {
    pub(super) target: ExecGrantTarget,
    pub(super) label: String,
    pub(super) device_id: String,
    pub(super) fingerprint: String,
    /// The room of a member.
    pub(super) room: Option<RoomFacts>,
    pub(super) exec: ExecGrant,
}

/// Every device of `profiles` whose rights can be set, direct ones first.
pub(super) fn targets(profiles: &ShareProfiles) -> Vec<AccessTarget> {
    let mut all = Vec::new();
    for grant in &profiles.direct_grants {
        if grant.state != crate::share::DirectGrantState::Accepted {
            continue;
        }
        let contact = profiles.direct_contacts.iter().find(|contact| {
            contact.remote_device_id.as_ref() == Some(&grant.device_id)
                && contact.remote_public_key.as_ref() == Some(&grant.public_key)
        });
        all.push(AccessTarget {
            target: ExecGrantTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            label: contact.map_or_else(|| grant.device_name.clone(), |c| c.display_name.clone()),
            device_id: grant.device_id.clone(),
            fingerprint: grant.fingerprint.clone(),
            room: None,
            exec: grant.exec.clone(),
        });
    }
    for room in profiles.rooms.iter().filter(|room| room.auto_join) {
        let members = room.members.iter().filter(|member| !member.blocked);
        all.extend(members.map(|member| member_target(room, member)));
    }
    all
}

fn member_target(room: &RoomProfile, member: &RoomMember) -> AccessTarget {
    AccessTarget {
        target: ExecGrantTarget::RoomMember {
            room_id: room.room_id.clone(),
            device_id: member.device_id.clone(),
            public_key: member.public_key.clone(),
            fingerprint: member.fingerprint.clone(),
            node_id: member.node_id.clone(),
        },
        label: format!("{} / {}", room.name, member.device_name),
        device_id: member.device_id.clone(),
        fingerprint: member.fingerprint.clone(),
        room: Some(RoomFacts::of(room)),
        exec: member.exec.clone(),
    }
}

/// Whether `selector` names `target`: its name, its id or fingerprint (or a
/// prefix of at least four characters), or its `exec://` target.
pub(super) fn names(target: &AccessTarget, selector: &str) -> bool {
    let lower = selector.to_ascii_lowercase();
    let prefix = |candidate: &str| {
        candidate.eq_ignore_ascii_case(selector)
            || (selector.len() >= 4 && candidate.to_ascii_lowercase().starts_with(&lower))
    };
    target.label.eq_ignore_ascii_case(selector)
        || prefix(&target.device_id)
        || prefix(&target.fingerprint)
        || output::selector(&target.target) == selector
}

/// The one device `selector` names; without a selector the only device.
pub(super) fn select_one<'a>(
    all: &'a [AccessTarget],
    selector: Option<&str>,
) -> Result<&'a AccessTarget, String> {
    let matches: Vec<&AccessTarget> = all
        .iter()
        .filter(|target| selector.is_none_or(|selector| names(target, selector)))
        .collect();
    match (matches.as_slice(), selector) {
        ([target], _) => Ok(target),
        ([], _) if all.is_empty() => {
            Err("no device to set access for: accept a direct request or join a room first".into())
        }
        ([], Some(selector)) => Err(format!(
            "no device matches {selector}; `se share grants access` lists the devices"
        )),
        _ => Err(format!(
            "several devices match; name one of: {}",
            matches
                .iter()
                .map(|target| output::selector(&target.target))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The admitted members of the joined room `selector`, as they are now.
pub(super) fn room_targets(
    profiles: &ShareProfiles,
    selector: &str,
) -> Result<(RoomFacts, Vec<AccessTarget>), String> {
    let room = super::super::rooms::find(profiles, selector)?;
    if !room.auto_join {
        return Err(format!(
            "room {} was left; join it again first: se share room rejoin {}",
            room.name, room.id
        ));
    }
    let admitted = room.members.iter().filter(|member| member.is_admitted());
    let members: Vec<AccessTarget> = admitted.map(|member| member_target(room, member)).collect();
    if members.is_empty() {
        return Err(format!(
            "room {} has no admitted member; `se share room list` shows who waits",
            room.name
        ));
    }
    Ok((RoomFacts::of(room), members))
}

/// A toggle without a selector considers only the devices it changes.
fn select<'a>(
    choices: &'a [AccessTarget],
    selector: Option<&str>,
    enabling: bool,
    changes_settings: bool,
) -> Result<&'a AccessTarget, String> {
    let matches = choices
        .iter()
        .filter(|choice| choice.exec.enabled != enabling || changes_settings)
        .filter(|choice| selector.is_none_or(|selector| names(choice, selector)))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [choice] => Ok(*choice),
        [] => Err("no matching Exec grant; run `se share grants exec` to list choices".into()),
        _ => Err(format!(
            "multiple Exec grants match; choose one: {}",
            matches
                .iter()
                .map(|choice| output::selector(&choice.target))
                .collect::<Vec<_>>()
                .join(", ")
        )),
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

    fn choice(label: &str, enabled: bool) -> AccessTarget {
        AccessTarget {
            target: ExecGrantTarget::Direct {
                device_id: label.into(),
                public_key: format!("{label}-key"),
                fingerprint: format!("{label}-fingerprint"),
                node_id: format!("{label}-node"),
            },
            label: label.into(),
            device_id: label.into(),
            fingerprint: format!("{label}-fingerprint"),
            room: None,
            exec: ExecGrant {
                enabled,
                ..ExecGrant::default()
            },
        }
    }
}
