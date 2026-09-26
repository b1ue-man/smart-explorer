//! Direct access requests (desktop lifecycle actions) and the one-shot
//! remote command (`daemon::exec_share`).
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{bool_arg, canceled, i64_arg, invalid, opt_bool, opt_str, str_arg};
use super::share_state::{default_home, identity, reconfigure};
use crate::mobile::{ApiError, Runtime, TaskCtx};
use crate::share::lifecycle_view::request_views;
use crate::share::{
    DirectDecisionKind, DirectRequestId, ExecRequest, PeerOpenTarget, ShareProfiles,
};

/// Output limit of a remote command (api.md §5).
const MAX_EXEC_OUTPUT: u64 = 1024 * 1024;
const MAX_EXEC_TIMEOUT_SECS: i64 = 24 * 60 * 60;
const EXEC_POLL: Duration = Duration::from_millis(250);

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

pub(super) fn exec(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
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
    let argv = if shell {
        vec![command.clone()]
    } else {
        split_command(&command).map_err(invalid)?
    };
    let request = ExecRequest {
        argv,
        cwd: (!path.trim_matches('/').is_empty()).then_some(path),
        timeout_ms: (timeout as u64).saturating_mul(1000),
        max_output_bytes: MAX_EXEC_OUTPUT,
        shell,
    };
    let task = rt.spawn_task("exec", format!("Befehl: {command}"), move |ctx| {
        exec_task(ctx, target, request)
    });
    Ok(json!({ "taskId": task }))
}

/// The remote call blocks until the command ends; it runs on a helper thread
/// so a cancel ends the task at once (the peer still enforces its timeout).
fn exec_task(
    ctx: &TaskCtx,
    target: PeerOpenTarget,
    request: ExecRequest,
) -> Result<Value, ApiError> {
    ctx.message("Befehl läuft…");
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("share-exec".into())
        .spawn(move || {
            let _ = tx.send(crate::daemon::exec_share(target, request));
        })
        .map_err(|error| super::args::io_error("Befehl starten", error))?;
    let result = loop {
        if ctx.cancelled() {
            return Err(canceled(
                "Abgebrochen (das Gerät beendet den Befehl nach seiner Zeitbegrenzung)",
            ));
        }
        match rx.recv_timeout(EXEC_POLL) {
            Ok(result) => break result,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Err(ApiError::new("internal", "Befehl ohne Ergebnis beendet."));
            }
        }
    };
    let output = result.map_err(exec_error)?;
    Ok(json!({
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exitCode": output.exit_code,
        "timedOut": output.timed_out,
        "truncated": output.stdout_truncated || output.stderr_truncated,
    }))
}

/// A refused grant is reported as such; everything else verbatim. Only grant
/// wording counts: a folder or file the device cannot access (an OS error,
/// an export) is no refused grant.
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
