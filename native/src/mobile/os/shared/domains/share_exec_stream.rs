//! `share.exec` over the background worker's Exec stream
//! (`daemon::connect_exec`), the path of `se exec`, `se shell` and MCP: the
//! contained process tree on the peer with its lifecycle, a confirmation the
//! peer's user may have to give first (PRM-M3) and the account the command
//! runs as. The one-shot request this replaces (`daemon::exec_share`, the
//! share service's `Ctrl::Exec`) is refused by every host
//! (`PreparedExec::run`), so no command from Android ran before. While the
//! start waits for the peer's user the task message says so; once authorized
//! it names the account („{Gerät}: als {Konto}, mit Rückfrage“, C6). Output
//! is kept up to 1 MiB (api.md §5); a cancel ends the remote process tree.
//!
//! Submodule of `share_requests.rs`.
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};
use serde_json::{json, Value};

use super::super::args::{canceled, io_error};
use super::{exec_error, MAX_EXEC_OUTPUT};
use crate::daemon::{
    ExecIpcEvent, ExecIpcFailure, ExecIpcSession, ExecNoticeView, ExecNoticeWatch,
};
use crate::mobile::{ApiError, TaskCtx};
use crate::share::exec_access_view::{ApprovalSurface, ExecAccessView};
use crate::share::exec_client_messages::{
    ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED, APPROVAL_CANCELLED, APPROVAL_DENIED,
    APPROVAL_UNAVAILABLE, PERMISSION_DENIED, RIGHT_NOT_GRANTED,
};
use crate::share::exec_helper_protocol::ACCOUNT_UNAVAILABLE;
use crate::share::{ExecProviderStatus, ExecStart, ExecTerminal, ExecTerminalKind, PeerOpenTarget};

const EXEC_POLL: Duration = Duration::from_millis(250);

/// Codes of a start the peer refused (C6): its grant, its user or the
/// account. They arrive as `permission`, the code first in the message, so
/// the app finds it („permission_denied: …“).
const REFUSALS: [&str; 8] = [
    PERMISSION_DENIED,
    RIGHT_NOT_GRANTED,
    APPROVAL_DENIED,
    APPROVAL_CANCELLED,
    APPROVAL_UNAVAILABLE,
    ACCOUNT_NOT_ALLOWED,
    ACCOUNT_HELPER_UNAVAILABLE,
    ACCOUNT_UNAVAILABLE,
];

/// What the stream told, from its reader thread.
enum Step {
    Authorized(ExecProviderStatus),
    Started,
    End(StreamEnd, Output),
}

enum StreamEnd {
    Terminal(ExecTerminal),
    Failed(ExecIpcFailure),
    /// The local stream to the worker broke.
    Broken(String),
}

/// stdout and stderr together up to `MAX_EXEC_OUTPUT` bytes.
#[derive(Default)]
pub(super) struct Output {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) truncated: bool,
}

impl Output {
    pub(super) fn push(&mut self, stderr: bool, bytes: &[u8]) {
        let used = self.stdout.len() + self.stderr.len();
        let room = usize::try_from(MAX_EXEC_OUTPUT)
            .unwrap_or(usize::MAX)
            .saturating_sub(used);
        let kept = bytes.len().min(room);
        if kept < bytes.len() {
            self.truncated = true;
        }
        let target = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        target.extend_from_slice(&bytes[..kept]);
    }
}

/// The task's message, set only when it changes.
struct Message<'a> {
    ctx: &'a TaskCtx,
    last: String,
}

impl Message<'_> {
    fn show(&mut self, text: &str) {
        if self.last != text {
            self.ctx.message(text);
            self.last = text.to_string();
        }
    }
}

/// Runs `start` on `target`; `label` names the device until its host does.
pub(super) fn run(
    ctx: &TaskCtx,
    target: PeerOpenTarget,
    start: ExecStart,
    label: &str,
) -> Result<Value, ApiError> {
    let mut message = Message {
        ctx,
        last: String::new(),
    };
    message.show(&connecting_line(label));
    let session = crate::daemon::connect_exec(target, start).map_err(exec_error)?;
    let exec_id = session.exec_id.clone();
    let input = session
        .take_input()
        .map_err(|error| io_error("Befehl starten", error))?;
    // `share.exec` sends no input: the command reads an empty stdin.
    input
        .eof()
        .map_err(|error| io_error("Befehl starten", error))?;
    let watch = ExecNoticeWatch::start(exec_id.clone(), |_| {});
    let steps = read_stream(session).map_err(|error| io_error("Befehl starten", error))?;
    let mut header: Option<String> = None;
    loop {
        if ctx.cancelled() {
            // The worker withdraws a waiting request or ends the process tree.
            let _ = input.cancel();
            return Err(canceled("Abgebrochen; das Gerät beendet den Befehl."));
        }
        match steps.recv_timeout(EXEC_POLL) {
            Ok(Step::Authorized(provider)) => {
                let notice = crate::daemon::exec_notice(&exec_id).ok().flatten();
                let line = account_line(
                    &device_of(notice.as_ref(), label),
                    notice.as_ref().and_then(|notice| notice.access.as_ref()),
                    &provider.user_label,
                );
                message.show(&line);
                header = Some(line);
            }
            Ok(Step::Started) => message.show(&running_line(header.as_deref())),
            Ok(Step::End(end, output)) => {
                return finish(ctx, end, &output, header.as_deref(), label)
            }
            Err(RecvTimeoutError::Timeout) => {
                if header.is_none() {
                    if let Some(line) = waiting_line_of(watch.latest().as_ref(), label) {
                        message.show(&line);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ApiError::internal("Befehl ohne Ergebnis beendet."));
            }
        }
    }
}

/// Reads the stream on its own thread until it ends; output stays there
/// and comes with the end, so a long output never queues up.
fn read_stream(mut session: ExecIpcSession) -> std::io::Result<Receiver<Step>> {
    let (steps, receiver) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("share-exec-stream".into())
        .spawn(move || {
            let mut output = Output::default();
            let end = loop {
                match session.next_event() {
                    Ok(ExecIpcEvent::Authorized(provider)) => {
                        let _ = steps.send(Step::Authorized(provider));
                    }
                    Ok(ExecIpcEvent::Started) => {
                        let _ = steps.send(Step::Started);
                    }
                    Ok(ExecIpcEvent::Stdout(bytes)) => output.push(false, &bytes),
                    Ok(ExecIpcEvent::Stderr(bytes)) => output.push(true, &bytes),
                    Ok(ExecIpcEvent::Terminal(terminal)) => break StreamEnd::Terminal(terminal),
                    Ok(ExecIpcEvent::Failed(failure)) => break StreamEnd::Failed(failure),
                    Err(error) => break StreamEnd::Broken(error.to_string()),
                }
            };
            // A cancelled task no longer listens.
            let _ = steps.send(Step::End(end, output));
        })?;
    Ok(receiver)
}

fn finish(
    ctx: &TaskCtx,
    end: StreamEnd,
    output: &Output,
    header: Option<&str>,
    label: &str,
) -> Result<Value, ApiError> {
    match end {
        StreamEnd::Terminal(terminal) => {
            let value = result_json(&terminal, output, header);
            match terminal_error(&terminal, label) {
                None => Ok(value),
                Some(error) => {
                    ctx.set_failure_result(value);
                    Err(error)
                }
            }
        }
        StreamEnd::Failed(failure) => Err(failure_error(&failure)),
        StreamEnd::Broken(error) => Err(ApiError::new(
            "network",
            format!("Verbindung zum Hintergrunddienst unterbrochen: {error}"),
        )),
    }
}

/// The task result (api.md §5, plus `signal` and the account line `header`).
pub(super) fn result_json(terminal: &ExecTerminal, output: &Output, header: Option<&str>) -> Value {
    json!({
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exitCode": terminal.exit_code,
        "timedOut": terminal.kind == ExecTerminalKind::TimedOut,
        "truncated": output.truncated || terminal.output_truncated,
        "signal": terminal.signal,
        "header": header,
    })
}

/// A command that exited or timed out is a result; every other end is an
/// error that names the device (`label`) and the host's reason.
pub(super) fn terminal_error(terminal: &ExecTerminal, label: &str) -> Option<ApiError> {
    let reason = terminal
        .message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| format!(" ({message})"))
        .unwrap_or_default();
    match terminal.kind {
        ExecTerminalKind::Exited | ExecTerminalKind::TimedOut => None,
        ExecTerminalKind::Cancelled => Some(ApiError::new(
            "conflict",
            format!("{label} hat den Befehl gestoppt{reason}."),
        )),
        ExecTerminalKind::Revoked => Some(ApiError::new(
            "permission",
            format!("{label} hat die Erlaubnis während des Befehls entzogen{reason}."),
        )),
        ExecTerminalKind::Disconnected => Some(ApiError::new(
            "network",
            format!("Verbindung zu {label} unterbrochen{reason}."),
        )),
        ExecTerminalKind::Failed => Some(ApiError::internal(format!(
            "Befehl auf {label} fehlgeschlagen{reason}."
        ))),
    }
}

/// A start that failed: refusals as `permission` with their code first.
pub(super) fn failure_error(failure: &ExecIpcFailure) -> ApiError {
    let kind = if REFUSALS.contains(&failure.code.as_str()) {
        "permission"
    } else {
        match failure.kind.as_str() {
            "authentication" => "permission",
            "local" => "internal",
            "remote" => "conflict",
            _ => "network",
        }
    };
    ApiError::new(kind, format!("{}: {}", failure.code, failure.message))
}

pub(super) fn connecting_line(label: &str) -> String {
    format!("Verbinde mit {label} …")
}

/// The status line while the start waits for the peer's user (C6).
pub(super) fn waiting_line(device: &str, surface: Option<ApprovalSurface>) -> String {
    let only_cli = if surface == Some(ApprovalSurface::CliOnly) {
        " (am Gerät nur mit „se share exec approvals“ zu bestätigen)"
    } else {
        ""
    };
    format!("Warte auf Bestätigung am Gerät {device} …{only_cli}")
}

/// The account line of the start (C6, „Kopfzeile“): `access` is `None` from
/// hosts up to 0.5.182, which name only their own account.
pub(super) fn account_line(
    device: &str,
    access: Option<&ExecAccessView>,
    user_label: &str,
) -> String {
    match access {
        Some(view) => {
            let ask = if view.asks() { ", mit Rückfrage" } else { "" };
            format!("{device}: als {}{ask}", view.account_label)
        }
        None => format!("{device}: Konto {user_label} (älteres Gerät, Rechte nicht gemeldet)"),
    }
}

pub(super) fn running_line(header: Option<&str>) -> String {
    match header {
        Some(header) => format!("{header} · Befehl läuft …"),
        None => "Befehl läuft …".to_string(),
    }
}

fn device_of(notice: Option<&ExecNoticeView>, label: &str) -> String {
    notice
        .map(|notice| notice.device.trim())
        .filter(|device| !device.is_empty())
        .map_or_else(|| label.to_string(), str::to_string)
}

fn waiting_line_of(notice: Option<&ExecNoticeView>, label: &str) -> Option<String> {
    let approval = notice?.approval.as_ref()?;
    Some(waiting_line(&device_of(notice, label), approval.surface()))
}

#[cfg(test)]
#[path = "share_exec_stream_tests.rs"]
mod tests;
