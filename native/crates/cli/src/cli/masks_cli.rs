//! `se masks`: mask lists for AI agents (plan 2026-11-28 S2, docs/oberflaechen/masken.md).
//! Lists, entries (literal, regex, fuzzy with a tolerance), protected folders,
//! lock and enabled state, the foreign MCP servers allowed during the agent
//! lockdown, and the state of the hardening. No command prints a value:
//! entries appear with id, kind, label and options only, values come from
//! stdin. A change that weakens the protection (unlock, disable, remove,
//! delete, change a value, allow a server) and every change of a locked list
//! asks the user through the system dialog of the elevation first, so an
//! agent that reaches `se masks` through a shell cannot weaken it.
#[path = "masks_cli_print.rs"]
mod print;
#[path = "masks_cli_store.rs"]
mod store;
#[cfg(test)]
#[path = "masks_cli_tests.rs"]
mod tests;

use super::hook::{hook_harden, hook_servers};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::io::Read;

#[derive(Args, Debug)]
pub(super) struct MasksArgs {
    #[command(subcommand)]
    command: MasksCommand,
}

#[derive(Args, Debug, Clone, Default)]
struct EntryOptions {
    #[arg(long, help = "Placeholder label: letters, digits, _ (default Maske)")]
    label: Option<String>,
    #[arg(
        long,
        help = "Fuzzy tolerance: off|low|medium|high (aus|gering|mittel|hoch) or <trim>/<inner errors>, e.g. 2/1.5"
    )]
    tolerance: Option<String>,
    #[arg(
        long,
        value_name = "true|false",
        help = "Literal: distinguish upper and lower case"
    )]
    case_sensitive: Option<bool>,
    #[arg(long, value_name = "true|false", help = "Literal: whole words only")]
    whole_word: Option<bool>,
}

#[derive(Subcommand, Debug)]
enum MasksCommand {
    #[command(about = "Lists with state, protected folders and entry counts (no values)")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "One list: entries with id, kind, label and options (no values)")]
    Show {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Create a list (enabled, unlocked, no entries)")]
    Create {
        name: String,
        #[arg(long)]
        json: bool,
    },
    #[command(
        about = "Add entries; the values come from stdin, one per line (--whole-input: all of stdin is one value)"
    )]
    Add {
        list: String,
        #[arg(long, default_value = "literal", help = "literal | regex | fuzzy")]
        kind: String,
        #[command(flatten)]
        options: EntryOptions,
        #[arg(long, help = "Take all of stdin as one value (multi-line regex)")]
        whole_input: bool,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Change an entry's options; --value-stdin replaces its value from stdin")]
    Update {
        list: String,
        entry: u64,
        #[command(flatten)]
        options: EntryOptions,
        #[arg(long)]
        value_stdin: bool,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Remove an entry (asks for confirmation)")]
    RemoveEntry {
        list: String,
        entry: u64,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Protected folders: add or remove (local path or se target, also remote)")]
    Scope {
        #[command(subcommand)]
        action: ScopeAction,
    },
    #[command(about = "Lock a list: agents can no longer add to it")]
    Lock {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Unlock a list (asks for confirmation)")]
    Unlock {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Enable a list")]
    Enable {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Disable a list (asks for confirmation)")]
    Disable {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Delete a list (asks for confirmation)")]
    Delete {
        list: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Foreign MCP servers agents may use during the lockdown")]
    Servers {
        #[command(subcommand)]
        action: ServerAction,
    },
    #[command(about = "Lockdown, hardening in Claude Code and Codex, allowed servers")]
    Status {
        #[arg(long)]
        json: bool,
    },
    #[command(
        about = "Write the hardening into the managed settings of Claude Code and Codex now (system dialog when it changes)"
    )]
    Harden {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ScopeAction {
    #[command(about = "Protect a folder")]
    Add {
        list: String,
        target: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Stop protecting a folder (asks for confirmation)")]
    Remove {
        list: String,
        target: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ServerAction {
    #[command(about = "Allowed servers")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Allow a server by its configured name (asks for confirmation)")]
    Allow {
        name: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Withdraw a server")]
    Remove {
        name: String,
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn run(args: MasksArgs) -> Result<i32, String> {
    let (value, json) = execute(args.command, &mut std::io::stdin().lock())?;
    print::emit(&value, json);
    Ok(0)
}

fn read_values(input: &mut dyn Read, whole: bool) -> Result<Vec<String>, String> {
    let mut text = String::new();
    input
        .read_to_string(&mut text)
        .map_err(|error| format!("stdin lesen: {error}"))?;
    if whole {
        let value = text
            .strip_suffix('\n')
            .map(|rest| rest.strip_suffix('\r').unwrap_or(rest))
            .unwrap_or(&text)
            .to_string();
        return Ok(if value.is_empty() {
            Vec::new()
        } else {
            vec![value]
        });
    }
    Ok(text
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn update_of(
    options: &EntryOptions,
    value: Option<String>,
) -> Result<crate::masks::EntryUpdate, String> {
    Ok(crate::masks::EntryUpdate {
        value,
        label: options.label.clone(),
        case_sensitive: options.case_sensitive,
        whole_word: options.whole_word,
        tolerance: options
            .tolerance
            .as_deref()
            .map(store::tolerance)
            .transpose()?,
    })
}

/// Runs one command; returns the result and whether JSON was asked for.
fn execute(command: MasksCommand, input: &mut dyn Read) -> Result<(Value, bool), String> {
    let refresh = |value: Value| -> Value {
        let mut value = value;
        value["hardening"] = hook_harden::refresh_if_needed();
        value
    };
    Ok(match command {
        MasksCommand::List { json } => (json!({"lists": store::Store::open()?.lists()?}), json),
        MasksCommand::Show { list, json } => {
            let store = store::Store::open()?;
            (json!({"list": store.find(&list)?}), json)
        }
        MasksCommand::Create { name, json } => {
            let store = store::Store::open()?;
            let id = store.create(&name)?;
            (json!({"created": store.summary(id)?}), json)
        }
        MasksCommand::Add {
            list,
            kind,
            options,
            whole_input,
            json,
        } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            let values = read_values(input, whole_input)?;
            if values.is_empty() {
                return Err("keine Werte auf stdin".into());
            }
            let entries = values
                .into_iter()
                .map(|value| store::new_entry(&kind, value, &options))
                .collect::<Result<Vec<_>, _>>()?;
            let added = store.add_entries(&summary, entries)?;
            (json!({"list": summary.id, "entries": added}), json)
        }
        MasksCommand::Update {
            list,
            entry,
            options,
            value_stdin,
            json,
        } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            let value = if value_stdin {
                read_values(input, true)?.into_iter().next()
            } else {
                None
            };
            let updated = store.update_entry(&summary, entry, update_of(&options, value)?)?;
            (json!({"list": summary.id, "entry": updated}), json)
        }
        MasksCommand::RemoveEntry { list, entry, json } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            store.remove_entry(&summary, entry)?;
            (json!({"list": summary.id, "removed_entry": entry}), json)
        }
        MasksCommand::Scope {
            action: ScopeAction::Add { list, target, json },
        } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            let scope = store.add_scope(&summary, &target)?;
            (
                refresh(json!({"list": summary.id, "added_scope": scope})),
                json,
            )
        }
        MasksCommand::Scope {
            action: ScopeAction::Remove { list, target, json },
        } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            store.remove_scope(&summary, &target)?;
            (
                refresh(json!({"list": summary.id, "removed_scope": target})),
                json,
            )
        }
        MasksCommand::Lock { list, json } => toggle(&list, store::Toggle::Lock, json)?,
        MasksCommand::Unlock { list, json } => toggle(&list, store::Toggle::Unlock, json)?,
        MasksCommand::Enable { list, json } => {
            let (value, json) = toggle(&list, store::Toggle::Enable, json)?;
            (refresh(value), json)
        }
        MasksCommand::Disable { list, json } => {
            let (value, json) = toggle(&list, store::Toggle::Disable, json)?;
            (refresh(value), json)
        }
        MasksCommand::Delete { list, json } => {
            let store = store::Store::open()?;
            let summary = store.find(&list)?;
            store.delete(&summary)?;
            (
                refresh(json!({"deleted": summary.id, "name": summary.name})),
                json,
            )
        }
        MasksCommand::Servers { action } => servers(action)?,
        MasksCommand::Status { json } => (status(), json),
        MasksCommand::Harden { json } => {
            (json!({"hardening": hook_harden::refresh_if_needed()}), json)
        }
    })
}

fn toggle(list: &str, toggle: store::Toggle, json: bool) -> Result<(Value, bool), String> {
    let store = store::Store::open()?;
    let summary = store.find(list)?;
    store.toggle(&summary, toggle)?;
    Ok((json!({"list": store.summary(summary.id)?}), json))
}

fn servers(action: ServerAction) -> Result<(Value, bool), String> {
    Ok(match action {
        ServerAction::List { json } => (
            json!({"allowed_mcp_servers": hook_servers::allowed()}),
            json,
        ),
        ServerAction::Allow { name, json } => {
            hook_servers::valid(&name)?;
            store::confirm(&format!(
                "Smart Explorer: dem Agenten während der Sperre den MCP-Server „{name}“ erlauben"
            ))?;
            (
                json!({"allowed_mcp_servers": hook_servers::change(&name, true)?}),
                json,
            )
        }
        ServerAction::Remove { name, json } => (
            json!({"allowed_mcp_servers": hook_servers::change(&name, false)?}),
            json,
        ),
    })
}

fn status() -> Value {
    let lockdown = match crate::masks::lockdown_state() {
        crate::masks::LockdownState::Off => "off",
        crate::masks::LockdownState::On => "on",
        crate::masks::LockdownState::Unknown => "unknown",
    };
    let lists = store::Store::open()
        .and_then(|store| store.lists())
        .map(|lists| json!(lists.len()))
        .unwrap_or_else(|error| json!({"error": error}));
    json!({
        "lockdown": lockdown,
        "lists": lists,
        "allowed_mcp_servers": hook_servers::allowed(),
        "hardening": hook_harden::status(),
    })
}
