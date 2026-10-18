//! `se share grants access` (PRM-M8, D.2.4): every right of one device in one
//! command (account, commands, terminal, confirmation, end, program list), or
//! the same change for every admitted member of a room as a snapshot: each
//! member gets a right of its own, later members get nothing. Without a
//! change option it shows the rights. The worker writes a change through the
//! grant journal at the revision this command read, so a concurrent change is
//! refused instead of overwritten. The device selection is the one of `se
//! share grants exec` (`grants_exec.rs`).
use std::collections::BTreeMap;

use clap::{ArgMatches, Args, Command, FromArgMatches};
use clap_complete::ArgValueCandidates;

use crate::cli::completions_access as completions;

use crate::share::access_catalog::{capability_catalog, HelperState, HelperStatus, HostFacts};
use crate::share::access_grant::{
    AccessGrant, AccessGrantChange, AccessPreset, AccountProfile, Capability, CapabilityDecision,
    Expiry,
};
use crate::share::ExecGrant;

use super::grants_access_output::{self as output, RoomFacts};
use super::grants_exec::{room_targets, select_one, targets, AccessTarget};

const ACCESS_HELP: &str = "\
Show or change what one device may do on this host, or what every admitted
member of a room may do. Without a change option the rights are shown.

Commands (programs and shell commands) and the terminal are each off, ask
(every start waits for a confirmation on this device) or allow. They run as
your own account (--as own), the restricted account se-remote (--as remote)
or another local account (--as account:NAME); the operating system enforces
what that account may do. --for or --until ends the right. With only --yes the
device gets commands and a terminal as your own account. --room ROOM
--all-members writes the change for every admitted member now; members
admitted later get nothing. A change that narrows a right ends the running
commands it no longer allows.

Examples:
  se share grants access Laptop --yes
  se share grants access Laptop --as remote --commands allow --terminal ask
  se share grants access --room Ops --all-members --ask --for 8h --yes
  se share grants access Laptop --off";

#[derive(Args)]
#[command(long_about = ACCESS_HELP)]
pub(super) struct AccessArgs {
    /// Device name, id or fingerprint prefix, or exec:// target; omit when only one device exists
    #[arg(conflicts_with = "room", add = ArgValueCandidates::new(completions::access_target_candidates))]
    pub(super) selector: Option<String>,
    /// Room profile id, room id or name: change every admitted member
    #[arg(long, value_name = "ROOM", requires = "all_members", add = ArgValueCandidates::new(completions::room_candidates))]
    pub(super) room: Option<String>,
    /// With --room: every admitted member
    #[arg(long, requires = "room")]
    pub(super) all_members: bool,
    /// Account commands and terminal run as
    #[arg(long = "as", value_name = "own|remote|account:NAME", add = ArgValueCandidates::new(completions::account_candidates))]
    pub(super) account: Option<String>,
    #[command(flatten)]
    pub(super) capabilities: CapabilityOptions,
    /// Every capability this change leaves allowed asks first
    #[arg(long)]
    pub(super) ask: bool,
    /// End the right after 30m, 8h, 2d or 1d12h
    #[arg(long = "for", value_name = "DURATION", conflicts_with_all = ["until", "no_expiry"])]
    pub(super) duration: Option<String>,
    /// End the right at this RFC 3339 time; without a zone the local time
    #[arg(long, value_name = "TIME", conflicts_with = "no_expiry")]
    pub(super) until: Option<String>,
    /// Remove the end of the right
    #[arg(long)]
    pub(super) no_expiry: bool,
    /// Allow only this program (repeatable): no shell command, no terminal
    #[arg(long = "allow-program", value_name = "PATH")]
    pub(super) allow_program: Vec<String>,
    /// Remove the program list
    #[arg(long, conflicts_with = "allow_program")]
    pub(super) full: bool,
    /// Switch commands and terminal off
    #[arg(long)]
    pub(super) off: bool,
    /// Confirm the access warning without a prompt
    #[arg(long)]
    pub(super) yes: bool,
}

/// One option per row of the capability catalog (`--commands`, `--terminal`;
/// the screen strand adds its rows there and gets its options with them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CapabilityOptions(pub(super) BTreeMap<Capability, CapabilityDecision>);

/// The rows this host offers: the screen rows (`--screen-view`,
/// `--screen-control`) only in a graphical session (C7).
fn offered() -> Vec<crate::share::access_catalog::CapabilityEntry> {
    let account_helper = HelperStatus {
        state: HelperState::NotNeeded,
        version: None,
        detail: None,
    };
    capability_catalog(&HostFacts {
        graphical_session: crate::screen::graphical_session(),
        account_helper,
    })
}

fn capability_help(capability: Capability) -> &'static str {
    match capability {
        Capability::ExecCommands => "Programs and shell commands: off, ask or allow",
        Capability::ExecTerminal => "A terminal with the login shell: off, ask or allow",
        Capability::ScreenView => "Watching the screen: off, ask or allow",
        Capability::ScreenControl => "Controlling the screen: off, ask or allow",
    }
}

impl FromArgMatches for CapabilityOptions {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let mut chosen = BTreeMap::new();
        for entry in offered() {
            let Ok(Some(value)) = matches.try_get_one::<String>(entry.cli_option) else {
                continue;
            };
            let decision = CapabilityDecision::from_key(value).ok_or_else(|| {
                let text = format!("--{} takes off, ask or allow\n", entry.cli_option);
                clap::Error::raw(clap::error::ErrorKind::InvalidValue, text)
            })?;
            chosen.insert(entry.capability, decision);
        }
        Ok(Self(chosen))
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

impl Args for CapabilityOptions {
    fn augment_args(command: Command) -> Command {
        offered().into_iter().fold(command, |command, entry| {
            command.arg(
                clap::Arg::new(entry.cli_option)
                    .long(entry.cli_option)
                    .value_name("off|ask|allow")
                    .value_parser(["off", "ask", "allow"])
                    .help(capability_help(entry.capability)),
            )
        })
    }

    fn augment_args_for_update(command: Command) -> Command {
        Self::augment_args(command)
    }
}

/// What the options ask for, before it is set against one grant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) account: Option<AccountProfile>,
    pub(super) decisions: BTreeMap<Capability, CapabilityDecision>,
    pub(super) off: bool,
    pub(super) ask: bool,
    pub(super) programs: Option<Vec<String>>,
    pub(super) expiry: Option<Expiry>,
    /// Only `--yes`: commands and terminal as the own account (D.3 T1).
    pub(super) full_own: bool,
}

/// The request of `args`; `None` shows the rights.
pub(super) fn request(args: &AccessArgs, now: i64) -> Result<Option<Request>, String> {
    if args.off && (args.ask || !args.capabilities.0.is_empty()) {
        return Err(
            "--off switches commands and terminal off; leave out --ask and the \
                    capability options"
                .into(),
        );
    }
    let expiry = match (&args.duration, &args.until, args.no_expiry) {
        (Some(duration), _, _) => Some(Expiry::At(
            now.saturating_add(output::parse_duration(duration)?),
        )),
        (_, Some(until), _) => Some(Expiry::At(output::parse_until(until, now)?)),
        (_, _, true) => Some(Expiry::None),
        _ => None,
    };
    let programs = match (args.full, args.allow_program.is_empty()) {
        (true, _) => Some(Vec::new()),
        (false, true) => None,
        (false, false) => Some(args.allow_program.clone()),
    };
    let request = Request {
        account: args.account.as_deref().map(parse_account).transpose()?,
        decisions: args.capabilities.0.clone(),
        off: args.off,
        ask: args.ask,
        programs,
        expiry,
        full_own: false,
    };
    if request != Request::default() {
        return Ok(Some(request));
    }
    let full_own = Request {
        full_own: true,
        ..Request::default()
    };
    Ok(args.yes.then_some(full_own))
}

impl Request {
    /// The change for one grant: a new program list allows commands that
    /// were off; `--ask` turns every capability the result allows into `ask`.
    pub(super) fn change_for(&self, current: &ExecGrant) -> AccessGrantChange {
        if self.full_own {
            return AccessPreset::FullOwn.change(Vec::new(), false);
        }
        let mut change = AccessGrantChange {
            account: self.account.clone(),
            capabilities: self.decisions.clone(),
            programs: self.programs.clone(),
            expiry: self.expiry,
        };
        let access = current.access_grant();
        if self.off {
            for capability in Capability::EXEC {
                change
                    .capabilities
                    .insert(capability, CapabilityDecision::Off);
            }
        }
        let listed = self.programs.as_ref().is_some_and(|list| !list.is_empty());
        if listed && !access.decision(Capability::ExecCommands).is_on() {
            let commands = change.capabilities.entry(Capability::ExecCommands);
            commands.or_insert(CapabilityDecision::Allow);
        }
        if self.ask {
            for capability in Capability::ALL {
                let result = change.capabilities.get(&capability).copied();
                if result.unwrap_or_else(|| access.decision(capability))
                    == CapabilityDecision::Allow
                {
                    change
                        .capabilities
                        .insert(capability, CapabilityDecision::Ask);
                }
            }
        }
        change
    }
}

/// The grant `change` makes of `current`, checked as the worker checks it.
pub(super) fn preview(
    current: &ExecGrant,
    change: &AccessGrantChange,
) -> Result<(AccessGrant, Vec<String>), String> {
    let mut grant = current.access_grant();
    let mut programs = current.programs.clone();
    grant.apply(change, &mut programs)?;
    Ok((grant, programs))
}

/// Whether going from `old` to `new` gives the device anything it did not
/// have: a higher decision, another account while commands or terminal are
/// on, the full shell or new programs, or a later or no end.
pub(super) fn widens(old: (&AccessGrant, &[String]), new: (&AccessGrant, &[String])) -> bool {
    let ((old, old_programs), (new, new_programs)) = (old, new);
    let raised = Capability::ALL
        .into_iter()
        .any(|capability| new.decision(capability).rank() > old.decision(capability).rank());
    let account = new.account != old.account && new.grants_exec();
    let programs = new.decision(Capability::ExecCommands).is_on()
        && new_programs != old_programs
        && (new_programs.is_empty() || new_programs.iter().any(|p| !old_programs.contains(p)));
    let later = new.grants_any()
        && match (old.valid_until, new.valid_until) {
            (Some(_), None) => true,
            (Some(before), Some(after)) => after > before,
            _ => false,
        };
    raised || account || programs || later
}

fn parse_account(text: &str) -> Result<AccountProfile, String> {
    let account = match (text, text.strip_prefix("account:")) {
        ("own", _) => AccountProfile::Own,
        ("remote", _) => AccountProfile::Remote,
        (_, Some(name)) => AccountProfile::Account { name: name.into() },
        _ => return Err(format!("--as {text}: use own, remote or account:NAME")),
    };
    account.validate().map(|()| account)
}

pub(super) fn run(args: AccessArgs, json: bool) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let request = request(&args, now)?;
    // `--room` comes only with `--all-members` (clap requires both).
    if let Some(room) = args.room.as_ref().filter(|_| args.all_members) {
        let (room, members) = room_targets(&profiles, room)?;
        return match request {
            None => show(&members, json, now),
            Some(request) => change_room(&room, &members, &request, args.yes, json),
        };
    }
    let all = targets(&profiles);
    let Some(request) = request else {
        return match args.selector.as_deref() {
            None => show(&all, json, now),
            selector => show_one(select_one(&all, selector)?, json, now),
        };
    };
    let target = select_one(&all, args.selector.as_deref())?;
    let change = request.change_for(&target.exec);
    confirm_if_wider(&[(target, &change)], &target.label, args.yes)?;
    let (value, line, complete) = apply(target, change)?;
    print(json, &value, std::slice::from_ref(&line))?;
    complete
        .then_some(())
        .ok_or_else(|| incomplete(&target.label))
}

/// The rights of every listed device: a JSON array, one line each.
fn show(targets: &[AccessTarget], json: bool, now: i64) -> Result<(), String> {
    let own = output::own_label();
    let values = targets.iter().map(|t| output::target_value(t, &own, now));
    let lines: Vec<String> = match targets {
        [] => vec!["access\t0\taccept a direct request or join a room first".into()],
        _ => targets
            .iter()
            .map(|t| output::target_line(t, &own, now))
            .collect(),
    };
    print(json, &serde_json::Value::Array(values.collect()), &lines)
}

/// The rights of the device a selector named: a JSON object, one line.
fn show_one(target: &AccessTarget, json: bool, now: i64) -> Result<(), String> {
    let own = output::own_label();
    let line = output::target_line(target, &own, now);
    print(json, &output::target_value(target, &own, now), &[line])
}

fn change_room(
    room: &RoomFacts,
    members: &[AccessTarget],
    request: &Request,
    yes: bool,
    json: bool,
) -> Result<(), String> {
    let changes: Vec<_> = members
        .iter()
        .map(|member| (member, request.change_for(&member.exec)))
        .collect();
    let planned: Vec<_> = changes
        .iter()
        .map(|(member, change)| (*member, change))
        .collect();
    let subject = format!("{} members of room {}", members.len(), room.name);
    confirm_if_wider(&planned, &subject, yes)?;
    let mut values = Vec::new();
    let mut lines = vec![format!(
        "room\t{}\tmembers={}",
        clean(&room.name),
        members.len()
    )];
    let mut failed = Vec::new();
    for (member, change) in changes {
        match apply(member, change) {
            Ok((value, line, complete)) => {
                values.push(value);
                lines.push(line);
                if !complete {
                    failed.push(member.label.clone());
                }
            }
            Err(error) => {
                lines.push(format!(
                    "access\t{}\terror={}",
                    clean(&member.label),
                    clean(&error)
                ));
                failed.push(member.label.clone());
            }
        }
    }
    print(json, &output::room_value(room, values), &lines)?;
    match failed.as_slice() {
        [] => Ok(()),
        _ => Err(format!(
            "the change is not complete for {}; run the command again, it reads the current \
             state first",
            failed.join(", ")
        )),
    }
}

/// Asks once when a change gives any device more than it had.
fn confirm_if_wider(
    planned: &[(&AccessTarget, &AccessGrantChange)],
    subject: &str,
    yes: bool,
) -> Result<(), String> {
    let mut wider = None;
    for (target, change) in planned {
        let (grant, programs) =
            preview(&target.exec, change).map_err(|error| format!("{}: {error}", target.label))?;
        let old = target.exec.access_grant();
        if widens((&old, &target.exec.programs), (&grant, &programs)) && wider.is_none() {
            wider = Some((grant, programs));
        }
    }
    match wider {
        Some((grant, programs)) if !yes => output::confirm_access(subject, &grant, &programs),
        _ => Ok(()),
    }
}

/// Sends one change: the value and line of its answer, and whether it is
/// stored and applied. A change that changes nothing (`--ask` for a device
/// without an allowed capability) is not sent.
fn apply(
    target: &AccessTarget,
    change: AccessGrantChange,
) -> Result<(serde_json::Value, String, bool), String> {
    if change.is_empty() {
        let line = format!("access\t{}\tunchanged", clean(&target.label));
        let value = output::change_value(target, serde_json::Value::Null, None);
        return Ok((value, line, true));
    }
    let revision = Some(target.exec.policy_revision);
    let (result, view) =
        crate::daemon::mutate_access_grant(target.target.clone(), change, revision)
            .map_err(|error| format!("{}: {error}{}", target.label, next_step(&error)))?;
    let complete = result.persisted && result.applied && result.error.is_none();
    let result = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    let line = output::change_line(target, &result, view.as_ref());
    Ok((
        output::change_value(target, result, view.as_ref()),
        line,
        complete,
    ))
}

fn print(json: bool, value: &serde_json::Value, lines: &[String]) -> Result<(), String> {
    if json {
        let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        lines.iter().for_each(|line| println!("{line}"));
    }
    Ok(())
}

/// The next step after a refused change (C6).
fn next_step(error: &str) -> &'static str {
    if error.contains("changed concurrently") {
        ". Next step: run the command again; it reads the current state first"
    } else if error.contains("another account") {
        ". Next step: choose --as own"
    } else {
        ""
    }
}

fn incomplete(label: &str) -> String {
    format!(
        "the change for {label} is fail-closed pending retry (see persisted and applied in the \
         answer); `se share grants access` shows the current state"
    )
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "grants_access_tests.rs"]
mod tests;
