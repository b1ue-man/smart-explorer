//! `se share exec approvals|approve|deny` (PRM-M8, D.2.4, C5): the requests
//! that wait for a confirmation on this host, and the host user's answer from
//! a terminal: „Diesmal erlauben“ (`approve`), „Immer erlauben“ (`approve
//! --always`, the worker sets the capability of that grant to `allow` before
//! the start continues) and „Ablehnen“ (`deny`). On a host without a
//! graphical session this is the only way to answer. An id may be shortened
//! to its first characters while exactly one open request starts with them.
//! The MCP tool `se` refuses `approve`, `deny` and the account setup (C7):
//! these decisions belong to a person at the device.
use std::io::Write;

use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::cli::completions_access::{approval_candidates, capability_name};
use crate::share::access_approval::{ApprovalCommand, ApprovalDecision, ApprovalView};

/// Shortest id prefix `approve` and `deny` resolve.
const MIN_PREFIX: usize = 4;
/// Length of a full request id (16 bytes as hex).
const ID_CHARS: usize = 32;
/// The refusal of the MCP tool `se` (C7).
const HUMAN_ONLY: &str = "Diese Entscheidung trifft nur ein Mensch am Gerät.";

#[derive(Subcommand)]
pub(super) enum ApprovalsCommand {
    #[command(about = "List the requests that wait for a confirmation on this host")]
    Approvals,
    #[command(about = "Allow one waiting request: this start, or with --always every later one")]
    Approve(ApproveArgs),
    #[command(about = "Refuse one waiting request")]
    Deny(DenyArgs),
}

#[derive(Args)]
pub(super) struct ApproveArgs {
    /// Id of the request from `se share exec approvals` (or its first characters)
    #[arg(add = ArgValueCandidates::new(approval_candidates))]
    pub(super) id: String,
    /// Allow this capability for the device from now on (no more questions)
    #[arg(long)]
    pub(super) always: bool,
}

#[derive(Args)]
pub(super) struct DenyArgs {
    /// Id of the request from `se share exec approvals` (or its first characters)
    #[arg(add = ArgValueCandidates::new(approval_candidates))]
    pub(super) id: String,
}

/// Where requests are read and answered: the background worker, or a test
/// service.
pub(super) trait ApprovalSource {
    fn pending(&self) -> Result<Vec<ApprovalView>, String>;
    fn decide(&self, id: &str, decision: ApprovalDecision) -> Result<(), String>;
}

/// The running background worker (`IpcRequest::Access`); without one no
/// request is open.
struct Worker;

impl ApprovalSource for Worker {
    fn pending(&self) -> Result<Vec<ApprovalView>, String> {
        crate::daemon::access_approvals()
    }

    fn decide(&self, id: &str, decision: ApprovalDecision) -> Result<(), String> {
        crate::daemon::decide_access_approval(id, decision)
    }
}

pub(super) fn run(command: ApprovalsCommand, json: bool) -> Result<(), String> {
    run_with(&Worker, command, json, &mut std::io::stdout().lock())
}

pub(super) fn run_with(
    source: &dyn ApprovalSource,
    command: ApprovalsCommand,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), String> {
    match command {
        ApprovalsCommand::Approvals => list(&source.pending()?, json, out),
        ApprovalsCommand::Approve(args) => {
            let decision = if args.always {
                ApprovalDecision::Always
            } else {
                ApprovalDecision::Once
            };
            answer(source, &args.id, decision, json, out)
        }
        ApprovalsCommand::Deny(args) => answer(source, &args.id, ApprovalDecision::Deny, json, out),
    }
}

fn write(out: &mut dyn Write, text: &str) -> Result<(), String> {
    writeln!(out, "{text}").map_err(|e| e.to_string())
}

fn list(pending: &[ApprovalView], json: bool, out: &mut dyn Write) -> Result<(), String> {
    if json {
        let text = serde_json::to_string_pretty(pending).map_err(|e| e.to_string())?;
        return write(out, &text);
    }
    if pending.is_empty() {
        return write(
            out,
            "approvals\t0\tno request waits for a confirmation on this host",
        );
    }
    for request in pending {
        write(out, &line(request))?;
    }
    write(out, "approve\tse share exec approve ID [--always]")?;
    write(out, "deny\tse share exec deny ID")
}

/// One request as a text line.
fn line(request: &ApprovalView) -> String {
    let room = request
        .room
        .as_deref()
        .map(|room| format!("\troom={room}"))
        .unwrap_or_default();
    format!(
        "approval\t{}\tdevice={}\tdevice_id={}\trelation={}{room}\tcapability={}\taccount={}\tcommand={}\trequested_at={}\tsurface={}",
        request.id,
        request.device_name,
        request.device_id,
        request.relation,
        capability_name(request.capability),
        request.account_label.as_deref().unwrap_or("-"),
        command_text(request.command.as_ref()),
        super::discoverable_output::local_time(request.requested_at),
        request.surface.as_deref().unwrap_or("-"),
    )
    .replace(['\r', '\n'], " ")
}

fn command_text(command: Option<&ApprovalCommand>) -> String {
    match command {
        None => "-".into(),
        Some(ApprovalCommand::Argv { program, args }) if args.is_empty() => program.clone(),
        Some(ApprovalCommand::Argv { program, args }) => format!("{program} {}", args.join(" ")),
        Some(ApprovalCommand::Shell { text }) => format!("shell: {text}"),
        Some(ApprovalCommand::Login) => "terminal (login shell)".into(),
    }
}

fn answer(
    source: &dyn ApprovalSource,
    given: &str,
    decision: ApprovalDecision,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), String> {
    let id = resolve(source, given.trim())?;
    source.decide(&id, decision).map_err(|error| {
        format!("{error}; `se share exec approvals` lists the requests that are still open")
    })?;
    let key = match decision {
        ApprovalDecision::Once => "once",
        ApprovalDecision::Always => "always",
        ApprovalDecision::Deny => "deny",
    };
    let text = if json {
        let value = serde_json::json!({"id": id, "decision": key, "decided": true});
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    } else {
        format!("approval\t{id}\tdecision={key}\tdecided=true")
    };
    write(out, &text)
}

/// The full id of the open request `given` names: the id itself, or the
/// only open request whose id starts with `given`.
fn resolve(source: &dyn ApprovalSource, given: &str) -> Result<String, String> {
    let hex = given.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !hex || given.len() < MIN_PREFIX || given.len() > ID_CHARS {
        return Err(format!(
            "{given} is no request id; `se share exec approvals` lists the open requests with \
             their ids"
        ));
    }
    let given = given.to_ascii_lowercase();
    if given.len() == ID_CHARS {
        return Ok(given);
    }
    let pending = source.pending()?;
    let matches: Vec<&ApprovalView> = pending
        .iter()
        .filter(|request| request.id.as_str().starts_with(&given))
        .collect();
    match matches.as_slice() {
        [request] => Ok(request.id.to_string()),
        [] => Err(format!(
            "no open request starts with {given}; `se share exec approvals` lists the open \
             requests"
        )),
        _ => Err(format!(
            "several open requests start with {given}; give more characters of the id"
        )),
    }
}

/// Why the MCP tool `se` refuses `arguments` (C7): answering a request and
/// setting up accounts need a person at the device. Reading stays open, and
/// `--help` only prints help.
pub(crate) fn mcp_refusal(arguments: &[String]) -> Option<String> {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return None;
    }
    let words: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| !argument.starts_with('-'))
        .collect();
    let human_only = matches!(
        words.as_slice(),
        ["share", "exec", "approve" | "deny", ..]
            | [
                "share",
                "exec",
                "accounts",
                "setup" | "allow" | "disallow" | "update" | "remove",
                ..
            ]
    );
    human_only.then(|| {
        format!(
            "{HUMAN_ONLY} Only a person at this device answers access requests and sets up \
             accounts; ask the user to run `se {}` in a terminal on this device.",
            arguments.join(" ")
        )
    })
}

#[cfg(test)]
#[path = "exec_approvals_tests.rs"]
mod tests;
