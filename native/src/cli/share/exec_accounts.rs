//! `se share exec accounts` (PRM-M8 accounts, Z1, D.2.4): "Ausführen als"
//! of this host with the state of its account service, and the setup of
//! the service as administrator in exactly the words of the setup modes of
//! both platforms (`cli_words`/`parse_words`): `setup [--remote-account]
//! [--allow ACCOUNT]... [--yes]`, `allow ACCOUNT`, `disallow ACCOUNT`,
//! `update`, `remove [--delete-remote-account]`. Only `setup` asks for the
//! typed word YES (or takes `--yes`): it installs a service with
//! administrator rights. Without those rights the CLI asks for them
//! (`pkexec` on a Linux terminal, UAC on Windows) or prints the command a
//! person runs as administrator. The list comes from the running
//! background worker and, without one, from this process, which runs as the
//! same user; listing never starts the worker. The MCP tool `se` may list,
//! never set up (C7, `exec_approvals::mcp_refusal`).
use std::io::{IsTerminal, Write};

use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::share::access_catalog::{HelperState, HelperStatus};
use crate::share::exec_helper_protocol::{AccountChoice, AccountChoices};
use crate::share::{ExecAccountsAction, ExecAccountsElevation, ExecAccountsOutcome};

#[derive(Args)]
pub(super) struct AccountsArgs {
    #[command(subcommand)]
    command: Option<AccountsCommand>,
}

#[derive(Subcommand)]
enum AccountsCommand {
    #[command(
        about = "Install or renew the account service (administrator) and allow accounts for this user"
    )]
    Setup(SetupArgs),
    #[command(about = "Allow one local account for this user's remote commands (administrator)")]
    Allow(AccountArg),
    #[command(
        about = "Take one local account away from this user's remote commands (administrator)"
    )]
    Disallow(AccountArg),
    #[command(
        about = "Renew the account service after an update of Smart Explorer (administrator)"
    )]
    Update,
    #[command(
        about = "Remove the account service; se-remote stays unless --delete-remote-account (administrator)"
    )]
    Remove(RemoveArgs),
}

#[derive(Args)]
struct SetupArgs {
    /// Create the restricted account se-remote and allow it
    #[arg(long)]
    remote_account: bool,
    /// Also allow this existing local account (repeatable)
    #[arg(long = "allow", value_name = "ACCOUNT")]
    allow: Vec<String>,
    /// Set up the service without the typed confirmation
    #[arg(long)]
    yes: bool,
}

#[derive(Args)]
struct AccountArg {
    /// A local account of this host
    #[arg(value_name = "ACCOUNT")]
    account: String,
}

#[derive(Args)]
struct RemoveArgs {
    /// Also delete the account se-remote with its home or profile
    #[arg(long)]
    delete_remote_account: bool,
}

pub(super) fn run(args: AccountsArgs, json: bool) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    let Some(command) = args.command else {
        return list(&current_choices(), json, &mut out);
    };
    let (action, yes) = action_of(command);
    if matches!(action, ExecAccountsAction::Setup { .. }) && !yes {
        confirm_setup(&action)?;
    }
    let elevation = if std::io::stdin().is_terminal() {
        ExecAccountsElevation::PkexecTerminal
    } else {
        ExecAccountsElevation::SudoCommandOnly
    };
    let outcome = crate::share::exec_accounts_apply(&action, elevation)
        .map_err(|error| format!("{}: {error}", action_name(&action)))?;
    report(&action, &outcome, json, &mut out)
}

/// The action of a command and whether `--yes` was given.
fn action_of(command: AccountsCommand) -> (ExecAccountsAction, bool) {
    match command {
        AccountsCommand::Setup(setup) => (
            ExecAccountsAction::Setup {
                remote_account: setup.remote_account,
                allow: setup.allow,
            },
            setup.yes,
        ),
        AccountsCommand::Allow(arg) => (
            ExecAccountsAction::Allow {
                account: arg.account,
            },
            false,
        ),
        AccountsCommand::Disallow(arg) => (
            ExecAccountsAction::Disallow {
                account: arg.account,
            },
            false,
        ),
        AccountsCommand::Update => (ExecAccountsAction::Update, false),
        AccountsCommand::Remove(remove) => (
            ExecAccountsAction::Remove {
                delete_remote_account: remove.delete_remote_account,
            },
            false,
        ),
    }
}

fn action_name(action: &ExecAccountsAction) -> &'static str {
    match action {
        ExecAccountsAction::Setup { .. } => "setup",
        ExecAccountsAction::Allow { .. } => "allow",
        ExecAccountsAction::Disallow { .. } => "disallow",
        ExecAccountsAction::Update => "update",
        ExecAccountsAction::Remove { .. } => "remove",
    }
}

/// The worker's list when one runs (its view decides the grants),
/// otherwise this process's.
fn current_choices() -> AccountChoices {
    match crate::daemon::access_accounts() {
        Ok((helper, choices)) => AccountChoices { helper, choices },
        Err(_) => crate::share::exec_account_choices(),
    }
}

fn write(out: &mut dyn Write, text: &str) -> Result<(), String> {
    writeln!(out, "{text}").map_err(|error| error.to_string())
}

/// `se share exec accounts --json` (D.2.4): `{"helper":{…},"choices":[…]}`.
pub(super) fn value(choices: &AccountChoices) -> Value {
    serde_json::to_value(choices).unwrap_or(Value::Null)
}

fn list(choices: &AccountChoices, json: bool, out: &mut dyn Write) -> Result<(), String> {
    if json {
        let text = serde_json::to_string_pretty(&value(choices)).map_err(|e| e.to_string())?;
        return write(out, &text);
    }
    for line in lines(choices) {
        write(out, &line)?;
    }
    Ok(())
}

/// The text form: the service, one line per choice, and the next step
/// where the service is missing or outdated.
pub(super) fn lines(choices: &AccountChoices) -> Vec<String> {
    let mut lines = vec![helper_line(&choices.helper)];
    lines.extend(choices.choices.iter().map(choice_line));
    match choices.helper.state {
        HelperState::NotInstalled => {
            lines.push("setup\tse share exec accounts setup --remote-account".into())
        }
        HelperState::Outdated => lines.push("update\tse share exec accounts update".into()),
        _ => {}
    }
    lines
}

fn helper_line(helper: &HelperStatus) -> String {
    let state = serde_json::to_value(helper.state)
        .ok()
        .and_then(|state| state.as_str().map(str::to_string))
        .unwrap_or_default();
    format!(
        "helper\tstate={state}\tversion={}\tdetail={}",
        helper.version.as_deref().unwrap_or("-"),
        clean(helper.detail.as_deref().unwrap_or("-")),
    )
}

fn choice_line(choice: &AccountChoice) -> String {
    let profile = serde_json::to_value(choice.profile)
        .ok()
        .and_then(|profile| profile.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut line = format!("account\t{profile}");
    match &choice.account {
        Some(account) => line.push_str(&format!("\taccount={account}")),
        None => line.push_str(&format!("\tlabel={}", clean(&choice.label))),
    }
    line.push_str(&format!("\tavailable={}", choice.available));
    if let Some(reason) = &choice.reason {
        line.push_str(&format!("\treason={}", clean(reason)));
    }
    if let Some(code) = choice.reason_code {
        let code = serde_json::to_value(code).unwrap_or(Value::Null);
        line.push_str(&format!("\treason_code={}", code.as_str().unwrap_or("-")));
    }
    if let Some(next_step) = &choice.next_step {
        line.push_str(&format!("\tnext_step={}", clean(next_step)));
    }
    if let (Some(elevated), Some(admin_group)) = (choice.elevated, choice.admin_group) {
        line.push_str(&format!("\televated={elevated}\tadmin_group={admin_group}"));
    }
    line
}

/// The answer of a setup action: the steps, or the command a person runs
/// as administrator (then the command fails, nothing was changed).
fn report(
    action: &ExecAccountsAction,
    outcome: &ExecAccountsOutcome,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), String> {
    let name = action_name(action);
    match outcome {
        ExecAccountsOutcome::Done(steps) => {
            if json {
                let value = json!({"action": name, "done": true, "steps": steps});
                return write(out, &value.to_string());
            }
            write(out, &format!("accounts\t{name}\tdone=true"))?;
            for step in steps {
                write(out, &format!("step\t{}", clean(step)))?;
            }
            Ok(())
        }
        ExecAccountsOutcome::NeedsAdministrator { command, reason } => {
            if json {
                let value = json!({
                    "action": name,
                    "done": false,
                    "needs_administrator": {"command": command, "reason": reason},
                });
                write(out, &value.to_string())?;
            } else {
                write(
                    out,
                    &format!("accounts\t{name}\tdone=false\treason={}", clean(reason)),
                )?;
                write(out, &format!("command\t{command}"))?;
            }
            Err(format!(
                "{name} needs administrator rights ({reason}); run the command above as \
                 administrator"
            ))
        }
    }
}

/// The one confirmation of `setup` (C6: the CLI keeps the typed YES).
fn confirm_setup(action: &ExecAccountsAction) -> Result<(), String> {
    let remote = matches!(
        action,
        ExecAccountsAction::Setup {
            remote_account: true,
            ..
        }
    );
    eprintln!(
        "WARNING: this installs the Smart Explorer account service, which runs with \
         administrator rights (root or LocalSystem) and starts remote commands as the local \
         accounts you allow{}.",
        if remote {
            "; it creates the restricted account se-remote"
        } else {
            ""
        }
    );
    eprintln!(
        "Devices then run commands with the rights of the account their grant names; what \
         that account may not do in the operating system, they may not do either."
    );
    eprint!("Type YES to set up: ");
    std::io::stderr()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    (answer.trim() == "YES").then_some(()).ok_or_else(|| {
        "the account service was not set up; --yes confirms without the prompt".to_string()
    })
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "exec_accounts_tests.rs"]
mod tests;
