//! `masks`: the mask lists as an agent may see and extend them (plan
//! 2026-11-28, S2; user 2026-11-28: „Maskenlisten sollen auch über den MCP
//! anlegbar/erweiterbar sein, sofern sie vom Nutzer nicht gesperrt wurden“
//! and „dürfen für den Agenten NICHT lesbar sein“). Every change runs as
//! `Actor::Agent`: the store refuses changes to locked lists and every
//! weakening change (unlock, disable, remove); no answer carries a stored
//! value, only kinds, labels, tolerances and the guarded paths, which the
//! mask layer masks like any other output.
use super::args::Args;
use super::response;
use super::root_path::{self, Place};
use super::session::Session;
use crate::masks::{Actor, EntryKind, ListId, ListSummary, NewEntry, Tolerance};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

pub(super) const DESCRIPTION: &str = "The user's mask lists on this device. Values of an active list never reach you: every Smart Explorer output shows them as placeholders (`⟦Label-N⟧`, a found variant `⟦Label-N.V⟧`, a literal `⟦` as `⟦⟦`), and placeholders in your inputs are restored to the real values. `list` shows every list with `enabled`, `locked`, its guarded `scopes` and its entries' `kind`, `label` and `tolerance`, never a value. `create` makes a list (`name`). `add` adds `entries` to a `list` (its id or name): `value`, `kind` (`literal`, default: the exact text, without regard to case unless `case_sensitive`, optionally `whole_word`; `regex`: a Rust regex, every distinct match gets its own placeholder; `fuzzy`: also spellings with separators, umlaut forms, typos and OCR errors after `tolerance` off, low, medium, high or `<trim>/<inner errors>`), `label` (letters, digits, _; default Maske). `add_path` makes the list guard a folder or file (`path`, any target; relative = below the working root): while an enabled list guards a path, agents may work only through Smart Explorer tools. Locked lists refuse every change; unlocking, disabling and removing lists, entries and paths are only for the user.";

pub(super) fn properties() -> Value {
    json!({
        "action": {"type": "string", "enum": ["list", "create", "add", "add_path"], "description": "list (default), create, add or add_path."},
        "name": {"type": "string", "description": "For create: the name of the new list."},
        "list": {"type": "string", "description": "For add and add_path: the list's id (as `list` shows it) or name."},
        "entries": {"type": "array", "minItems": 1, "description": "For add: the entries to add.", "items": {
            "type": "object",
            "properties": {
                "value": {"type": "string", "description": "The text, regex or fuzzy word to mask."},
                "kind": {"type": "string", "enum": ["literal", "regex", "fuzzy"], "description": "literal (default), regex or fuzzy."},
                "label": {"type": "string", "description": "Placeholder label: letters, digits and _ (default Maske)."},
                "tolerance": {"type": "string", "description": "fuzzy only: off, low, medium (default), high or `<trim>/<inner errors>` such as `2/1.5`."},
                "case_sensitive": {"type": "boolean", "description": "literal only: distinguish upper and lower case (default false)."},
                "whole_word": {"type": "boolean", "description": "literal only: whole words only (default false)."}
            },
            "required": ["value"],
            "additionalProperties": false
        }},
        "path": {"type": "string", "description": "For add_path: the path to guard, any Smart Explorer target; relative = below the working root."}
    })
}

pub(super) fn masks(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let answer = match args.opt_string("action")?.as_deref().unwrap_or("list") {
        "list" => {
            let lists = session.masks.store(|store| store.lists().map_err(text))?;
            json!({ "lists": lists.iter().map(list_value).collect::<Vec<_>>() })
        }
        "create" => {
            let name = args.string("name")?;
            let id = session
                .masks
                .store(|store| store.create_list(Actor::Agent, &name).map_err(text))?;
            session.masks.invalidate();
            json!({ "created": true, "list": changed(session, id)? })
        }
        "add" => {
            let entries = parse_entries(args.value("entries"))?;
            let id = list_id(session, &args.string("list")?)?;
            let added = session
                .masks
                .store(|store| store.add_entries(Actor::Agent, id, entries).map_err(text))?;
            session.masks.invalidate();
            json!({
                "added": added.len(),
                "entries": serde_json::to_value(&added).map_err(|error| error.to_string())?,
                "list": changed(session, id)?,
            })
        }
        "add_path" => {
            let path = absolute(session, &args.string("path")?)?;
            let id = list_id(session, &args.string("list")?)?;
            let guarded = session
                .masks
                .store(|store| store.add_scope(Actor::Agent, id, &path).map_err(text))?;
            session.masks.invalidate();
            // A new guarded path changes the agent hosts' managed rules (P6):
            // anchor them now, as `se masks scope add` and the settings page do.
            let hardening = crate::cli::hook::hook_harden::refresh_if_needed();
            json!({ "guarded": guarded, "list": changed(session, id)?, "hardening": hardening })
        }
        other => {
            return Err(format!(
                "unknown action `{other}`; use list, create, add or add_path"
            ))
        }
    };
    Ok(response::ok(answer))
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// A list as `list` shows it: its summary, which holds no value.
fn list_value(list: &ListSummary) -> Value {
    serde_json::to_value(list).unwrap_or_else(|error| json!({ "error": error.to_string() }))
}

/// The summary of list `id` after a change.
fn changed(session: &Session, id: ListId) -> Result<Value, String> {
    let lists = session.masks.store(|store| store.lists().map_err(text))?;
    Ok(lists
        .iter()
        .find(|list| list.id == id)
        .map(list_value)
        .unwrap_or(Value::Null))
}

/// The list a selector names: its id, else its name (without regard to
/// case, as the store compares names).
fn list_id(session: &Session, selector: &str) -> Result<ListId, String> {
    let selector = selector.trim();
    let found = session.masks.store(|store| {
        if let Ok(number) = selector.parse::<u64>() {
            let lists = store.lists().map_err(text)?;
            if let Some(list) = lists.iter().find(|list| list.id.0 == number) {
                return Ok(Some(list.id));
            }
        }
        store.find_list(selector).map_err(text)
    })?;
    found.ok_or_else(|| format!("no mask list `{selector}`; `masks` with action list shows them"))
}

/// The absolute target of `spec`, below the working root when relative.
fn absolute(session: &Session, spec: &str) -> Result<String, String> {
    match root_path::place(spec, session.targets.root())? {
        Place::Under(root, rel) => Ok(root_path::join_spec(&root.spec, &rel)),
        Place::Absolute(spec) => Ok(spec),
    }
}

pub(super) fn parse_entries(value: Option<&Value>) -> Result<Vec<NewEntry>, String> {
    let items = value
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "`entries` must be a non-empty list".to_string())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let at = |message: String| format!("entry {}: {message}", index + 1);
            let field = |name: &str| item.get(name).filter(|value| !value.is_null());
            let string = |name: &str| -> Result<Option<String>, String> {
                match field(name) {
                    None => Ok(None),
                    Some(Value::String(text)) => Ok(Some(text.clone())),
                    Some(_) => Err(at(format!("`{name}` must be a string"))),
                }
            };
            let flag = |name: &str| -> Result<bool, String> {
                match field(name) {
                    None => Ok(false),
                    Some(Value::Bool(value)) => Ok(*value),
                    Some(_) => Err(at(format!("`{name}` must be true or false"))),
                }
            };
            let value = string("value")?
                .filter(|value| !value.is_empty())
                .ok_or_else(|| at("`value` must be a non-empty string".into()))?;
            let kind = match string("kind")? {
                None => EntryKind::Literal,
                Some(kind) => EntryKind::parse(&kind)
                    .ok_or_else(|| at(format!("unknown kind `{kind}`; use literal, regex or fuzzy")))?,
            };
            let tolerance = match (kind, string("tolerance")?) {
                (EntryKind::Fuzzy, None) => Tolerance::Medium,
                (EntryKind::Fuzzy, Some(level)) => Tolerance::parse(&level).ok_or_else(|| {
                    at(format!(
                        "unknown tolerance `{level}`; use off, low, medium, high or <trim>/<inner errors>"
                    ))
                })?,
                (_, Some(_)) => return Err(at("`tolerance` applies to fuzzy entries only".into())),
                (_, None) => Tolerance::Off,
            };
            let case_sensitive = flag("case_sensitive")?;
            let whole_word = flag("whole_word")?;
            if kind != EntryKind::Literal && (case_sensitive || whole_word) {
                return Err(at(
                    "`case_sensitive` and `whole_word` apply to literal entries only".into(),
                ));
            }
            Ok(NewEntry {
                kind,
                value,
                label: string("label")?,
                case_sensitive,
                whole_word,
                tolerance,
            })
        })
        .collect()
}
