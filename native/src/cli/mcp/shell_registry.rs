//! The shell sessions of one `se mcp` server: sessions it opened (owner) and
//! command sessions of other processes it attached to (`sessions` → attach,
//! e.g. a `se shell -T --attachable` of the user). Each keeps a bounded log of
//! what the other participants did there, for `sessions` → activity. Terminal
//! sessions of `se shell` are listed with their kind but never joined: what
//! an agent typed there would reach the program the user is running.
use super::targets::Targets;
use crate::cli::shell_session::{self, Activity, OpenOptions, SessionClient, SessionKind};
use crate::creds::SavedConnection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Activity text kept per session; older text is dropped and counted.
const MAX_ACTIVITY_BYTES: usize = 64 * 1024;
/// Shared wait for own sessions to end when the server stops, inside its
/// 10-second close grace and the updater's 30-second wait.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

pub(super) struct Port {
    pub(super) client: SessionClient,
    /// This server opened the session (closing ends it).
    pub(super) owned: bool,
    /// Remote identity of an own session, to refuse a different `target`.
    target_key: Option<String>,
    log: Arc<Mutex<ActivityLog>>,
    _operation: Arc<Mutex<Option<crate::mcp_launcher::OperationGuard>>>,
}

#[derive(Default)]
struct ActivityLog {
    text: String,
    dropped: u64,
}

type Slot = Arc<Mutex<Option<Arc<Port>>>>;

#[derive(Default)]
pub(super) struct ShellRegistry {
    slots: Mutex<BTreeMap<String, Slot>>,
}

impl ShellRegistry {
    /// The live session `name`, opened on `target` when it does not exist
    /// or has ended. A given `target` must name the session's remote.
    pub(super) fn get_or_open(
        &self,
        name: &str,
        target: Option<&str>,
        attachable: bool,
        targets: &Targets,
    ) -> Result<Arc<Port>, String> {
        shell_session::validate_name(name)?;
        let target = target.map(str::trim).filter(|value| !value.is_empty());
        let slot = self.slot(name)?;
        let mut current = slot.lock().map_err(|_| poisoned())?;
        if let Some(port) = current
            .as_ref()
            .filter(|port| port.client.ended().is_none())
        {
            if let Some(target) = target {
                check_target(name, port, target)?;
            }
            return Ok(port.clone());
        }
        let target = target.ok_or_else(|| {
            format!("no session named `{name}` is open; give `target` to open it")
        })?;
        let spec = shell_session::resolve_target(Some(target))?;
        let client = shell_session::open(
            &spec,
            &|connection: &SavedConnection| targets.ssh(connection),
            OpenOptions {
                name: name.to_string(),
                owner: owner_label(),
                attachable,
                cwd: None,
                env: BTreeMap::new(),
            },
        )?;
        let port = Arc::new(Port::new(client, true, Some(spec.key()))?);
        *current = Some(port.clone());
        Ok(port)
    }

    /// Joins an attachable command session of another process by name or id.
    pub(super) fn attach(&self, selector: &str) -> Result<Arc<Port>, String> {
        let entry = shell_session::find_attachable(selector)?;
        agent_may_join(entry.kind, &entry.name)?;
        let slot = self.slot(&entry.name)?;
        let mut current = slot.lock().map_err(|_| poisoned())?;
        if let Some(port) = current
            .as_ref()
            .filter(|port| port.client.ended().is_none())
        {
            if port.client.info().id == entry.id {
                return Ok(port.clone());
            }
            return Err(format!(
                "this server already uses a session named `{}`; close it first",
                entry.name
            ));
        }
        let client = SessionClient::attach(&entry, &owner_label())?;
        let port = Arc::new(Port::new(client, false, None)?);
        *current = Some(port.clone());
        Ok(port)
    }

    /// Ends an own session (waits up to `CLOSE_WAIT`) or detaches from an
    /// attached one; returns whether the session was own.
    pub(super) fn close(&self, name: &str) -> Result<bool, String> {
        let slot = self
            .slots
            .lock()
            .map_err(|_| poisoned())?
            .remove(name)
            .ok_or_else(|| format!("no session named `{name}` in this server"))?;
        let port = slot
            .lock()
            .map_err(|_| poisoned())?
            .take()
            .ok_or_else(|| format!("no session named `{name}` in this server"))?;
        port.client.leave();
        if port.owned {
            port.client.wait_ended(CLOSE_WAIT);
        }
        Ok(port.owned)
    }

    /// This server's sessions and the user's other attachable sessions.
    pub(super) fn list(&self) -> Vec<Value> {
        let mut rows = Vec::new();
        let mut joined = BTreeSet::new();
        for (name, port) in self.ports() {
            let info = port.client.info();
            joined.insert(info.id.clone());
            let activity_bytes = port.log.lock().map(|log| log.text.len()).unwrap_or(0);
            rows.push(json!({
                "session": name,
                "id": info.id,
                "kind": SessionKind::Commands.label(),
                "role": if port.owned { "owner" } else { "attached" },
                "target": info.target,
                "dialect": info.dialect,
                "cwd": info.cwd,
                "owner": info.owner,
                "attachable": info.attachable,
                "ended": port.client.ended().map(|(_, message)| message),
                "activity_bytes": activity_bytes,
            }));
        }
        for entry in shell_session::list_attachable() {
            if joined.contains(&entry.id) {
                continue;
            }
            rows.push(attachable_row(entry.public_value(), entry.kind));
        }
        rows
    }

    /// What the other participants did since the last call.
    pub(super) fn activity(&self, name: &str) -> Result<Value, String> {
        let port = self
            .ports()
            .into_iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, port)| port)
            .ok_or_else(|| format!("no session named `{name}` in this server"))?;
        let mut log = port.log.lock().map_err(|_| poisoned())?;
        let text = std::mem::take(&mut log.text);
        let dropped = std::mem::take(&mut log.dropped);
        Ok(json!({ "session": name, "activity": text, "dropped_bytes": dropped }))
    }

    /// Server end: leave every session, then give own ones `CLOSE_WAIT`.
    /// A session still being opened is not waited for; it ends with this
    /// process.
    pub(super) fn close_all(&self) {
        let slots: Vec<Slot> = match self.slots.lock() {
            Ok(mut slots) => std::mem::take(&mut *slots).into_values().collect(),
            Err(_) => Vec::new(),
        };
        let mut ports = Vec::new();
        for slot in slots {
            if let Ok(mut port) = slot.try_lock() {
                ports.extend(port.take());
            }
        }
        for port in &ports {
            port.client.leave();
        }
        let deadline = Instant::now() + CLOSE_WAIT;
        for port in ports.iter().filter(|port| port.owned) {
            port.client
                .wait_ended(deadline.saturating_duration_since(Instant::now()));
        }
    }

    fn slot(&self, name: &str) -> Result<Slot, String> {
        Ok(self
            .slots
            .lock()
            .map_err(|_| poisoned())?
            .entry(name.to_string())
            .or_default()
            .clone())
    }

    fn ports(&self) -> Vec<(String, Arc<Port>)> {
        let slots: Vec<(String, Slot)> = match self.slots.lock() {
            Ok(slots) => slots
                .iter()
                .map(|(name, slot)| (name.clone(), slot.clone()))
                .collect(),
            Err(_) => Vec::new(),
        };
        slots
            .into_iter()
            .filter_map(|(name, slot)| {
                // A session still being opened is skipped, not waited for.
                let port = slot.try_lock().ok()?.clone()?;
                Some((name, port))
            })
            .collect()
    }
}

impl Port {
    fn new(client: SessionClient, owned: bool, target_key: Option<String>) -> Result<Self,String> {
        let operation=Arc::new(Mutex::new(crate::mcp_launcher::continuation_operation()?));
        let active=operation.clone();
        let log = Arc::new(Mutex::new(ActivityLog::default()));
        let events = client.activity();
        let writer = log.clone();
        let logging = std::thread::Builder::new()
            .name("se-mcp-shell-log".into())
            .spawn(move || {
                for event in events {
                    let ended=matches!(event,Activity::Ended {..});
                    if let Ok(mut log) = writer.lock() {
                        log.push(&event);
                    }
                    if ended {if let Ok(mut guard)=active.lock() {guard.take();}}
                }
                if let Ok(mut guard)=active.lock() {guard.take();}
            });
        if let Err(error)=logging {client.leave(); return Err(format!("cannot watch persistent shell lifetime: {error}"));}
        Ok(Self {
            _operation:operation,
            client,
            owned,
            target_key,
            log,
        })
    }
}

impl ActivityLog {
    fn push(&mut self, activity: &Activity) {
        let text = match activity {
            Activity::Started { by, command } => format!("[{by}] $ {command}\n"),
            Activity::Output { bytes, .. } => String::from_utf8_lossy(bytes).into_owned(),
            Activity::Done { by, exit_code, cwd } => format!(
                "[{by}] exit {} in {}\n",
                exit_code.map_or_else(|| "?".to_string(), |code| code.to_string()),
                cwd.as_deref().unwrap_or("?")
            ),
            Activity::Notice(text) => format!("[se] {text}\n"),
            Activity::Ended { message, .. } => format!("[se] session ended: {message}\n"),
        };
        self.text.push_str(&text);
        if self.text.len() > MAX_ACTIVITY_BYTES {
            let mut cut = self.text.len() - MAX_ACTIVITY_BYTES;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.dropped += cut as u64;
            self.text.drain(..cut);
        }
    }
}

/// Agents join only command sessions. Input typed into a terminal session
/// would reach whatever the user is running there (an editor, `sudo`, a
/// REPL) and the text they are typing; the user can still share a shell with
/// an agent by starting it as `se shell TARGET -T --attachable`.
fn agent_may_join(kind: SessionKind, name: &str) -> Result<(), String> {
    match kind {
        SessionKind::Commands => Ok(()),
        SessionKind::Terminal => Err(format!(
            "session `{name}` is a full terminal session of `se shell`; agents can join only command sessions: ask the user to start `se shell TARGET -T --attachable`"
        )),
    }
}

/// The listing row of another process's attachable session: what the
/// registry publishes (including `kind`) plus how to proceed.
fn attachable_row(mut row: Value, kind: SessionKind) -> Value {
    let next = match kind {
        SessionKind::Commands => "sessions with action attach joins it",
        SessionKind::Terminal => "a terminal session of the user; agents cannot join it",
    };
    if let Some(fields) = row.as_object_mut() {
        fields.insert("role".into(), json!("none"));
        fields.insert("next".into(), json!(next));
    }
    row
}

/// A given `target` must name the session's remote: a command must never
/// run on another machine than the caller expects.
fn check_target(name: &str, port: &Port, target: &str) -> Result<(), String> {
    let info = port.client.info();
    let same = match &port.target_key {
        Some(key) => shell_session::resolve_target(Some(target))
            .map(|spec| &spec.key() == key)
            .unwrap_or(false),
        None => target == info.target,
    };
    if same {
        Ok(())
    } else {
        Err(format!(
            "session `{name}` runs on {}, not on {target}; omit `target` or use another session name",
            info.target
        ))
    }
}

fn owner_label() -> String {
    format!("se mcp (pid {})", std::process::id())
}

fn poisoned() -> String {
    "the shell session registry lock is poisoned".to_string()
}

#[cfg(test)]
mod tests {
    use super::{agent_may_join, attachable_row, ActivityLog};
    use crate::cli::shell_session::{Activity, SessionKind};
    use serde_json::json;

    #[test]
    fn mcp_task_terminal_sessions_refuse_agents() {
        assert!(agent_may_join(SessionKind::Commands, "build").is_ok());
        let refused = agent_may_join(SessionKind::Terminal, "work").unwrap_err();
        assert!(refused.contains("`work`"), "{refused}");
        assert!(refused.contains("terminal"), "{refused}");
        assert!(refused.contains("-T --attachable"), "{refused}");
    }

    #[test]
    fn mcp_task_listed_sessions_of_the_user_say_whether_agents_can_join() {
        let command = attachable_row(
            json!({"session": "build", "kind": "commands"}),
            SessionKind::Commands,
        );
        assert_eq!(command["kind"], "commands");
        assert_eq!(command["role"], "none");
        assert_eq!(command["next"], "sessions with action attach joins it");
        let terminal = attachable_row(
            json!({"session": "work", "kind": "terminal"}),
            SessionKind::Terminal,
        );
        assert_eq!(terminal["kind"], "terminal");
        assert_eq!(terminal["role"], "none");
        assert_eq!(
            terminal["next"],
            "a terminal session of the user; agents cannot join it"
        );
    }

    #[test]
    fn mcp_task_session_activity_log_is_bounded() {
        let mut log = ActivityLog::default();
        log.push(&Activity::Started {
            by: "se shell (pid 7)".into(),
            command: "make".into(),
        });
        log.push(&Activity::Done {
            by: "se shell (pid 7)".into(),
            exit_code: Some(2),
            cwd: Some("/srv".into()),
        });
        assert_eq!(
            log.text,
            "[se shell (pid 7)] $ make\n[se shell (pid 7)] exit 2 in /srv\n"
        );
        log.push(&Activity::Output {
            stream: crate::cli::shell_session::Stream::Stdout,
            bytes: "é".repeat(40_000).into_bytes(),
        });
        assert!(log.text.len() <= super::MAX_ACTIVITY_BYTES);
        assert!(log.dropped > 0);
    }
}
