//! Direct access requests (desktop lifecycle actions) and the remote command
//! `share.exec`, which runs over the background worker's Exec stream like
//! `se exec` (`share_exec_stream.rs`).
use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::args::{bool_arg, i64_arg, invalid, opt_bool, opt_str, str_arg};
use super::share_state::{default_home, identity, reconfigure, with_state};
use crate::mobile::{ApiError, Runtime};
use crate::share::lifecycle_view::request_views;
use crate::share::{
    DirectDecisionKind, DirectRequestId, ExecCommand, ExecId, ExecStart, PeerOpenTarget,
    ShareProfiles,
};

#[path = "share_exec_stream.rs"]
mod exec_stream;

/// Output limit of a remote command (api.md §5).
const MAX_EXEC_OUTPUT: u64 = 1024 * 1024;
const MAX_EXEC_TIMEOUT_SECS: i64 = 24 * 60 * 60;

fn request_id(args: &Value) -> Result<DirectRequestId, ApiError> {
    DirectRequestId::parse(str_arg(args, "requestId")?)
        .map_err(|_| invalid("Ungültige Anfrage-ID."))
}

pub(super) fn request_access(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let contact_id = str_arg(args, "contactId")?;
    let message = opt_str(args, "message")
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string);
    let identity = identity()?;
    crate::share::queue_direct_request_for_contact(default_home(), &identity, contact_id, message)
        .map_err(|error| {
            ApiError::new(
                "invalid",
                format!("Direkt-Anfrage nicht vorgemerkt: {error}"),
            )
        })?;
    reconfigure(rt);
    Ok(json!({}))
}

pub(super) fn decide(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = request_id(args)?;
    let accept = bool_arg(args, "accept")?;
    let profiles = ShareProfiles::load_checked(default_home())
        .map_err(|error| ApiError::new("internal", error))?;
    let (incoming, _) = request_views(&profiles, super::args::now_secs());
    let view = incoming
        .iter()
        .find(|view| view.request_id == id)
        .ok_or_else(|| ApiError::new("not_found", "Anfrage nicht gefunden."))?;
    if !view.can_decide || (accept && !view.can_accept) {
        return Err(ApiError::new(
            "conflict",
            if view.identity_conflict {
                "Identitätskonflikt: Annehmen ist gesperrt."
            } else {
                "Über diese Anfrage kann nicht mehr entschieden werden."
            },
        ));
    }
    let decision = if accept {
        DirectDecisionKind::Accepted
    } else {
        DirectDecisionKind::Rejected
    };
    let identity = identity()?;
    crate::share::decide_direct_request(
        default_home(),
        &identity,
        &id,
        &view.fingerprint,
        decision,
        None,
    )
    .map_err(|error| {
        ApiError::new(
            "conflict",
            format!("Direkt-Entscheidung nicht gespeichert: {error}"),
        )
    })?;
    reconfigure(rt);
    Ok(json!({}))
}

pub(super) fn retry(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = request_id(args)?;
    crate::share::retry_direct_request_now(default_home(), &id).map_err(|error| {
        ApiError::new(
            "conflict",
            format!("Direkt-Anfrage nicht erneut vorgemerkt: {error}"),
        )
    })?;
    reconfigure(rt);
    Ok(json!({}))
}

pub(super) fn delete_request(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = request_id(args)?;
    crate::share::delete_direct_request_history(default_home(), &id)
        .map_err(|error| ApiError::new("conflict", format!("Anfrage nicht gelöscht: {error}")))?;
    reconfigure(rt);
    Ok(json!({}))
}

/// Splits a command line into arguments: whitespace separates, single and
/// double quotes group. A backslash escapes whitespace or a quote outside
/// quotes and a double quote inside double quotes; any other backslash stays
/// (Windows paths such as `C:\Users` or `\\server\share`).
pub(super) fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some('\''), c) => current.push(c),
            (_, '\\') => match (quote, chars.peek().copied()) {
                (Some(_), Some('"')) => {
                    current.push('"');
                    chars.next();
                }
                (None, Some(next)) if next.is_whitespace() || next == '"' || next == '\'' => {
                    current.push(next);
                    chars.next();
                }
                _ => current.push('\\'),
            },
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if quote.is_some() {
        return Err("Anführungszeichen im Befehl sind nicht geschlossen.".to_string());
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

/// `share.exec {location, command, shell, timeoutSecs}` → `{taskId}`. The
/// task runs the command over the background worker's Exec stream, the path
/// of `se exec` (`share_exec_stream.rs`); the one-shot request it replaces
/// (`daemon::exec_share`) is refused by every host. Result: `{stdout, stderr,
/// exitCode, timedOut, truncated, signal, header}`.
pub(super) fn exec(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let (target, start, command) = exec_start(args)?;
    let label = peer_label(&target);
    let task = rt.spawn_task("exec", format!("Befehl: {command}"), move |ctx| {
        exec_stream::run(ctx, target, start, &label)
    });
    Ok(json!({ "taskId": task }))
}

/// The checked arguments of `share.exec` as the Exec start of a command:
/// `shell` runs the text through the device's shell, otherwise it is split
/// into program and arguments (`split_command`). The location names the
/// device itself: a folder in it would reach the host as a working directory
/// of its own file system, which a Share path is not. Stdin stays empty, the
/// output is limited to 1 MiB, and the device ends the command after
/// `timeoutSecs`.
pub(super) fn exec_start(args: &Value) -> Result<(PeerOpenTarget, ExecStart, String), ApiError> {
    let location = str_arg(args, "location")?;
    let command = str_arg(args, "command")?.trim().to_string();
    let shell = opt_bool(args, "shell", false);
    let timeout = i64_arg(args, "timeoutSecs")?;
    if !(1..=MAX_EXEC_TIMEOUT_SECS).contains(&timeout) {
        return Err(invalid(
            "Die Zeitbegrenzung muss zwischen 1 s und 24 h liegen.",
        ));
    }
    if command.is_empty() {
        return Err(invalid("Bitte einen Befehl eingeben."));
    }
    let (target, path) = PeerOpenTarget::from_endpoint(location)
        .ok_or_else(|| invalid("Befehle lassen sich nur auf Share-Geräten ausführen."))?;
    if !path.trim_matches('/').is_empty() {
        return Err(invalid(
            "Befehle starten im Arbeitsordner des Geräts; bitte das Gerät selbst angeben, keinen Ordner darin.",
        ));
    }
    let spec = if shell {
        ExecCommand::Shell {
            command: command.clone(),
        }
    } else {
        let mut argv = split_command(&command).map_err(invalid)?;
        if argv.first().is_none_or(|program| program.trim().is_empty()) {
            return Err(invalid("Bitte einen Befehl eingeben."));
        }
        let program = argv.remove(0);
        ExecCommand::Argv {
            program,
            args: argv,
        }
    };
    let start = ExecStart {
        exec_id: ExecId::generate()
            .map_err(|error| ApiError::internal(format!("Befehls-ID nicht erzeugt: {error}")))?,
        command: spec,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: Some((timeout as u64).saturating_mul(1000)),
        max_output_bytes: Some(MAX_EXEC_OUTPUT),
        terminal: None,
    };
    start
        .validate()
        .map_err(|error| invalid(format!("Befehl ungültig: {error}")))?;
    Ok((target, start, command))
}

/// How the task names the device until its host reports a name: the name
/// this phone stored for the contact or room member, else „Gerät“.
fn peer_label(target: &PeerOpenTarget) -> String {
    with_state(|state| {
        state
            .profiles
            .as_ref()
            .and_then(|profiles| profile_name(profiles, target))
    })
    .unwrap_or_else(|| "Gerät".to_string())
}

pub(super) fn profile_name(profiles: &ShareProfiles, target: &PeerOpenTarget) -> Option<String> {
    let name = match target {
        PeerOpenTarget::Direct { contact_id } => profiles
            .direct_contacts
            .iter()
            .find(|contact| &contact.id == contact_id)
            .map(|contact| contact.display_name.clone()),
        PeerOpenTarget::RoomDevice { room_id, device_id } => profiles
            .rooms
            .iter()
            .find(|room| &room.id == room_id || &room.room_id == room_id)
            .and_then(|room| {
                room.members
                    .iter()
                    .find(|member| &member.device_id == device_id)
                    .map(|member| member.device_name.clone())
            }),
    };
    name.filter(|name| !name.trim().is_empty())
}

/// A refused grant is reported as such; everything else verbatim. Only grant
/// wording counts: a folder or file the device cannot access (an OS error,
/// an export) is no refused grant. Used for the errors of starting the
/// stream; the stream's own refusals carry their code
/// (`share_exec_stream.rs`).
pub(super) fn exec_error(error: String) -> ApiError {
    let lower = error.to_lowercase();
    const REFUSED: [&str; 3] = ["grant", "exec-freigabe", "befehlsfreigabe"];
    const NO_ACCESS: [&str; 3] = [
        "permission denied",
        "access is denied",
        "zugriff verweigert",
    ];
    if REFUSED.iter().any(|needle| lower.contains(needle)) {
        ApiError::new(
            "permission",
            format!("Gerät erlaubt keine Befehle von diesem Telefon ({error})"),
        )
    } else if NO_ACCESS.iter().any(|needle| lower.contains(needle)) {
        ApiError::new("permission", error)
    } else {
        ApiError::new("network", error)
    }
}
