//! The Codex user hook of the mask list lockdown in `config.toml`: one
//! `[[hooks.PreToolUse]]` group that runs `se hook codex`, plus its
//! `[hooks.state."<key>"] trusted_hash`, because Codex skips a non-managed
//! hook until its exact definition is trusted (CX-src
//! `hooks/src/engine/discovery.rs`; docs/refs/agenten-sperre-elevation-2026-11-28.md
//! §2.1) and the user does no manual steps. Only the group se recorded (by
//! fingerprint) or a group that runs `… hook codex` is ever replaced or
//! removed; every other hook, state entry, comment and table stays.
#[cfg(test)]
#[path = "codex_hooks_tests.rs"]
mod codex_hooks_tests;
use crate::cli::hook::hook_install;
use serde_json::{json, Value};
use std::path::Path;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

fn hooks_table(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    if doc.get("hooks").is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        doc["hooks"] = Item::Table(table);
    }
    doc["hooks"]
        .as_table_mut()
        .ok_or_else(|| "integration_conflict: Codex `hooks` is not a table".to_string())
}

fn owned_index(groups: &ArrayOfTables, owner: Option<&Value>, executable: &Path) -> Option<usize> {
    let recorded = owner.and_then(|owner| owner["hook"]["fingerprint"].as_str());
    let (command, _) = hook_install::codex_commands(executable);
    groups.iter().position(|group| {
        let fingerprint = hook_install::codex_group_fingerprint(group);
        (recorded.is_some() && fingerprint.as_deref() == recorded)
            || (hook_install::is_se_codex_group(group)
                && hook_install::codex_group_command(group) == Some(command.as_str()))
    })
}

/// Adds or updates the se group and its trust entry; returns the ownership
/// record kept in the Codex owner file. `doc` is unchanged on an error.
pub(super) fn apply(
    doc: &mut DocumentMut,
    config: &Path,
    executable: &Path,
    owner: Option<&Value>,
) -> Result<Value, String> {
    let program = hook_install::hook_program(executable);
    let executable = program.as_path();
    let mut staged = doc.clone();
    let hooks = hooks_table(&mut staged)?;
    if hooks.get("PreToolUse").is_none() {
        hooks.insert("PreToolUse", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let groups = hooks["PreToolUse"]
        .as_array_of_tables_mut()
        .ok_or_else(|| {
            "integration_conflict: Codex `hooks.PreToolUse` is not an array of tables".to_string()
        })?;
    let group = hook_install::codex_group(executable);
    let fingerprint = hook_install::codex_group_fingerprint(&group)
        .ok_or("integration_codex_hook_fingerprint_failed")?;
    let index = match owned_index(groups, owner, executable) {
        Some(index) => {
            if let Some(slot) = groups.get_mut(index) {
                *slot = group;
            }
            index
        }
        None => {
            groups.push(group);
            groups.len() - 1
        }
    };
    let (command, command_windows) = hook_install::codex_commands(executable);
    let effective = if cfg!(windows) {
        command_windows
    } else {
        command
    };
    let trusted_hash = hook_install::codex_trust_hash(&effective);
    let key = hook_install::codex_state_key(config, index);
    if hooks.get("state").is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        hooks.insert("state", Item::Table(table));
    }
    let state = hooks["state"]
        .as_table_mut()
        .ok_or("integration_conflict: Codex `hooks.state` is not a table")?;
    // A moved group leaves the trust entry se wrote for its old position.
    if let Some(old) = owner.and_then(|owner| owner["hook"]["state_key"].as_str()) {
        let recorded = owner.and_then(|owner| owner["hook"]["trusted_hash"].as_str());
        if old != key && state_trusted(state, old) == recorded {
            remove_trust(state, old);
        }
    }
    if state.get(&key).is_none() {
        state.insert(&key, Item::Table(Table::new()));
    }
    let entry = state[key.as_str()]
        .as_table_mut()
        .ok_or("integration_conflict: Codex hook state entry is not a table")?;
    entry.insert("trusted_hash", toml_edit::value(trusted_hash.as_str()));
    *doc = staged;
    Ok(json!({"fingerprint":fingerprint,"state_key":key,"trusted_hash":trusted_hash}))
}

fn state_trusted<'a>(state: &'a Table, key: &str) -> Option<&'a str> {
    state.get(key)?.get("trusted_hash")?.as_str()
}

fn remove_trust(state: &mut Table, key: &str) {
    let Some(entry) = state.get_mut(key).and_then(Item::as_table_mut) else {
        return;
    };
    entry.remove("trusted_hash");
    if entry.is_empty() {
        state.remove(key);
    }
}

/// Removes the se group and the trust entry se wrote; `true` when `doc`
/// changed.
pub(super) fn remove(doc: &mut DocumentMut, owner: Option<&Value>) -> bool {
    let Some(hooks) = doc.get_mut("hooks").and_then(Item::as_table_mut) else {
        return false;
    };
    let mut changed = false;
    let recorded = owner.and_then(|owner| owner["hook"]["fingerprint"].as_str());
    if let Some(groups) = hooks
        .get_mut("PreToolUse")
        .and_then(Item::as_array_of_tables_mut)
    {
        let before = groups.len();
        groups.retain(|group| {
            let fingerprint = hook_install::codex_group_fingerprint(group);
            !((recorded.is_some() && fingerprint.as_deref() == recorded)
                || hook_install::is_se_codex_group(group))
        });
        changed |= groups.len() != before;
        if groups.is_empty() {
            hooks.remove("PreToolUse");
        }
    }
    if let (Some(key), Some(state)) = (
        owner.and_then(|owner| owner["hook"]["state_key"].as_str()),
        hooks.get_mut("state").and_then(Item::as_table_mut),
    ) {
        let recorded = owner.and_then(|owner| owner["hook"]["trusted_hash"].as_str());
        if recorded.is_some() && state_trusted(state, key) == recorded {
            remove_trust(state, key);
            changed = true;
        }
        if state.is_empty() {
            hooks.remove("state");
        }
    }
    if hooks.is_empty() {
        doc.remove("hooks");
    }
    changed
}

/// Hook part of `se plugin status` for Codex.
pub(super) fn status(doc: &DocumentMut, config: &Path, executable: &Path) -> Value {
    let program = hook_install::hook_program(executable);
    let executable = program.as_path();
    let groups = doc
        .get("hooks")
        .and_then(|hooks| hooks.get("PreToolUse"))
        .and_then(Item::as_array_of_tables);
    let Some((index, group)) = groups.and_then(|groups| {
        groups
            .iter()
            .enumerate()
            .find(|(_, group)| hook_install::is_se_codex_group(group))
    }) else {
        return json!({"present":false});
    };
    let (command, command_windows) = hook_install::codex_commands(executable);
    let effective = if cfg!(windows) {
        command_windows
    } else {
        command
    };
    let current = hook_install::codex_group_fingerprint(group)
        == hook_install::codex_group_fingerprint(&hook_install::codex_group(executable));
    let key = hook_install::codex_state_key(config, index);
    let trusted = doc
        .get("hooks")
        .and_then(|hooks| hooks.get("state"))
        .and_then(Item::as_table)
        .and_then(|state| state_trusted(state, &key))
        == Some(hook_install::codex_trust_hash(&effective).as_str());
    json!({"present":true,"current":current,"trusted":trusted})
}
