//! Background work of the page „Maskenlisten“: the changes on the mask store
//! (`crate::masks::MaskStore`) as the desktop user, the system dialog for a
//! weakening change, and the `se masks` runs for the hardening and the
//! allowed MCP servers (the CLI owns them, so desktop and terminal write the
//! same files and ask the same dialog).
use crate::masks::{
    Actor, EntryId, EntryUpdate, EntryWithValue, ListId, ListSummary, MaskError, MaskStore,
    NewEntry,
};
use crossbeam_channel::Receiver;
use serde_json::Value;

/// One change the page asks for.
#[derive(Clone, Debug)]
pub(in crate::app) enum Op {
    Create(String),
    Add(ListId, NewEntry),
    Update(ListId, EntryId, EntryUpdate),
    RemoveEntry(ListId, EntryId),
    AddScope(ListId, String),
    RemoveScope(ListId, String),
    Lock(ListId, bool),
    Enable(ListId, bool),
    Delete(ListId),
    /// Clear text of one entry (only this desktop surface may read it).
    Reveal(ListId, EntryId),
    AllowServer(String),
    RemoveServer(String),
    Harden,
}

/// What a finished operation reports.
#[derive(Debug, Default)]
pub(in crate::app) struct Done {
    pub(in crate::app) message: Option<Result<String, String>>,
    pub(in crate::app) revealed: Option<(EntryId, String)>,
    pub(in crate::app) reload: bool,
}

impl Op {
    /// Whether the change can alter the protected local folders of the
    /// enabled lists, so the hardening is brought up to date afterwards.
    pub(in crate::app) fn touches_scopes(&self) -> bool {
        matches!(
            self,
            Op::AddScope(..) | Op::RemoveScope(..) | Op::Enable(..) | Op::Delete(..)
        )
    }

    /// Text of the system dialog for a weakening change.
    pub(in crate::app) fn reason(&self, list: &str) -> String {
        let what = match self {
            Op::Create(_) | Op::Add(..) | Op::Reveal(..) | Op::Harden => "ändern",
            Op::Update(..) => "Eintrag ändern",
            Op::RemoveEntry(..) => "Eintrag entfernen",
            Op::AddScope(..) => "Schutzpfad hinzufügen",
            Op::RemoveScope(..) => "Schutzpfad freigeben",
            Op::Lock(_, true) => "sperren",
            Op::Lock(_, false) => "entsperren",
            Op::Enable(_, true) => "aktivieren",
            Op::Enable(_, false) => "deaktivieren",
            Op::Delete(_) => "löschen",
            Op::AllowServer(_) | Op::RemoveServer(_) => "MCP-Server ändern",
        };
        format!("Smart Explorer: Maskenliste „{list}“ {what}")
    }
}

pub(in crate::app) fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<Receiver<T>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .ok()
        .map(|_| rx)
}

pub(in crate::app) fn load() -> Result<Vec<ListSummary>, String> {
    MaskStore::open_default()
        .and_then(|store| store.lists())
        .map_err(|error| error.to_string())
}

/// Runs `change` as the desktop user; a change the store refuses as
/// weakening runs again after the system dialog as the verified user.
fn as_user<T>(reason: &str, change: impl Fn(Actor) -> Result<T, MaskError>) -> Result<T, String> {
    match change(Actor::Gui) {
        Err(MaskError::NotAllowed(_)) => {
            crate::elevate::confirm_user_presence(reason).map_err(|error| error.to_string())?;
            change(Actor::VerifiedUser).map_err(|error| error.to_string())
        }
        other => other.map_err(|error| error.to_string()),
    }
}

fn store_op(
    store: &MaskStore,
    op: &Op,
    reason: &str,
) -> Result<(String, Option<(EntryId, String)>), String> {
    Ok(match op {
        Op::Create(name) => {
            store
                .create_list(Actor::Gui, name)
                .map_err(|error| error.to_string())?;
            (format!("Maskenliste „{name}“ angelegt."), None)
        }
        Op::Add(list, entry) => {
            as_user(reason, |actor| {
                store.add_entries(actor, *list, vec![entry.clone()])
            })?;
            ("Eintrag hinzugefügt.".into(), None)
        }
        Op::Update(list, entry, update) => {
            as_user(reason, |actor| {
                store.update_entry(actor, *list, *entry, update.clone())
            })?;
            ("Eintrag geändert.".into(), None)
        }
        Op::RemoveEntry(list, entry) => {
            as_user(reason, |actor| store.remove_entry(actor, *list, *entry))?;
            ("Eintrag entfernt.".into(), None)
        }
        Op::AddScope(list, scope) => {
            let scope = as_user(reason, |actor| store.add_scope(actor, *list, scope))?;
            (format!("Schutzpfad „{scope}“ hinzugefügt."), None)
        }
        Op::RemoveScope(list, scope) => {
            as_user(reason, |actor| store.remove_scope(actor, *list, scope))?;
            (format!("Schutzpfad „{scope}“ freigegeben."), None)
        }
        Op::Lock(list, locked) => {
            as_user(reason, |actor| store.set_locked(actor, *list, *locked))?;
            (
                if *locked {
                    "Liste gesperrt."
                } else {
                    "Liste entsperrt."
                }
                .into(),
                None,
            )
        }
        Op::Enable(list, enabled) => {
            as_user(reason, |actor| store.set_enabled(actor, *list, *enabled))?;
            (
                if *enabled {
                    "Liste aktiviert."
                } else {
                    "Liste deaktiviert."
                }
                .into(),
                None,
            )
        }
        Op::Delete(list) => {
            as_user(reason, |actor| store.delete_list(actor, *list))?;
            ("Liste gelöscht.".into(), None)
        }
        Op::Reveal(list, entry) => {
            let entries: Vec<EntryWithValue> = store
                .reveal(Actor::Gui, *list)
                .map_err(|error| error.to_string())?;
            let value = entries
                .into_iter()
                .find(|item| item.entry.id == *entry)
                .map(|item| item.value)
                .ok_or_else(|| "Der Eintrag ist nicht mehr in der Liste.".to_string())?;
            (String::new(), Some((*entry, value)))
        }
        Op::AllowServer(_) | Op::RemoveServer(_) | Op::Harden => (String::new(), None),
    })
}

/// Runs one change on a worker thread.
pub(in crate::app) fn run(op: Op, list_name: String) -> Done {
    let reason = op.reason(&list_name);
    let mut done = Done {
        reload: !matches!(op, Op::Reveal(..)),
        ..Done::default()
    };
    let cli = match &op {
        Op::AllowServer(name) => Some(vec![
            "masks".into(),
            "servers".into(),
            "allow".into(),
            name.clone(),
            "--json".into(),
        ]),
        Op::RemoveServer(name) => Some(vec![
            "masks".into(),
            "servers".into(),
            "remove".into(),
            name.clone(),
            "--json".into(),
        ]),
        Op::Harden => Some(vec!["masks".into(), "harden".into(), "--json".into()]),
        _ => None,
    };
    if let Some(arguments) = cli {
        done.message = Some(run_se(&arguments).map(|value| summary(&value)));
        return done;
    }
    let result = MaskStore::open_default()
        .map_err(|error| error.to_string())
        .and_then(|store| store_op(&store, &op, &reason));
    match result {
        Ok((message, revealed)) => {
            done.revealed = revealed;
            let mut message = message;
            if op.touches_scopes() {
                match run_se(&["masks".into(), "harden".into(), "--json".into()]) {
                    Ok(value) => {
                        let text = summary(&value);
                        if !text.is_empty() {
                            message = format!("{message} {text}");
                        }
                    }
                    Err(error) => message = format!("{message} Härtung: {error}"),
                }
            }
            if !message.is_empty() {
                done.message = Some(Ok(message));
            }
        }
        Err(error) => done.message = Some(Err(error)),
    }
    done
}

/// Short text of a `se masks … --json` result for the page.
pub(in crate::app) fn summary(value: &Value) -> String {
    let hardening = &value["hardening"];
    if let Some(error) = hardening["error"].as_str() {
        return format!("Härtung: {error}");
    }
    match hardening["state"].as_str() {
        Some("applied") => "Härtung in Claude Code und Codex verankert.".into(),
        Some("current") => "Härtung ist aktuell.".into(),
        Some("not_needed") => String::new(),
        Some("failed") => format!(
            "Härtung nicht verankert: {}",
            hardening["error"].as_str().unwrap_or("unbekannter Fehler")
        ),
        _ if value.get("allowed_mcp_servers").is_some() => "Erlaubte MCP-Server geändert.".into(),
        _ => String::new(),
    }
}

/// `se masks status --json`.
pub(in crate::app) fn status() -> Result<Value, String> {
    run_se(&["masks".into(), "status".into(), "--json".into()])
}

/// Runs the installed `se` beside the app and parses its JSON output.
fn run_se(arguments: &[String]) -> Result<Value, String> {
    let se = crate::updater::installed_cli_path()?;
    if !se.is_file() {
        return Err(format!("se nicht gefunden: {}", se.display()));
    }
    let mut command = std::process::Command::new(&se);
    command
        .args(arguments)
        .env_remove("COMPLETE")
        .stdin(std::process::Stdio::null());
    crate::app::hide_child_console(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("{} starten: {error}", se.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("Ausgabe von se: {error}"))
}
