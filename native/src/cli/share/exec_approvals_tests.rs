//! Acceptance of `se share exec approvals|approve|deny` against the approval
//! service of this host, and of what the MCP tool `se` refuses (PRM-M8, C7).
use std::sync::Arc;

use serde_json::{json, Value};

use super::{mcp_refusal, run_with, ApprovalSource, ApprovalsCommand, ApproveArgs, DenyArgs};
use crate::share::access_approval::{
    ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalOutcome, ApprovalService,
    ApprovalSummary, ApprovalTicket, ApprovalView, ConsentNotifier,
};
use crate::share::access_grant::Capability;
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::relation_proof::ProvenPrincipal;

/// A host without a graphical session: only the CLI answers.
struct CliOnly;

impl ConsentNotifier for CliOnly {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        Some(ApprovalSurface::CliOnly)
    }
}

/// The service in this process stands in for the worker's.
impl ApprovalSource for ApprovalService {
    fn pending(&self) -> Result<Vec<ApprovalView>, String> {
        Ok(ApprovalService::pending(self))
    }

    fn decide(&self, id: &str, decision: ApprovalDecision) -> Result<(), String> {
        ApprovalService::decide(self, &ApprovalId::parse(id)?, decision)
    }
}

fn ask(service: &ApprovalService, device: &str, command: &str) -> ApprovalTicket {
    let principal = ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: String::new(),
        device_id: device.into(),
        device_name: format!("Name {device}"),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("key-{device}"),
    };
    let summary = ApprovalSummary {
        account_label: Some("alice".into()),
        command: Some(ApprovalCommand::Shell {
            text: command.into(),
        }),
        policy_revision: 3,
        ..ApprovalSummary::default()
    };
    service.request(&principal, Capability::ExecCommands, summary)
}

fn outcome(ticket: &mut ApprovalTicket) -> ApprovalOutcome {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(ticket.outcome())
}

fn run(source: &ApprovalService, command: ApprovalsCommand, json: bool) -> Result<String, String> {
    let mut out = Vec::new();
    run_with(source, command, json, &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}

fn listed(source: &ApprovalService) -> Value {
    serde_json::from_str(&run(source, ApprovalsCommand::Approvals, true).unwrap()).unwrap()
}

fn id_of(source: &ApprovalService, device: &str) -> String {
    let pending = ApprovalService::pending(source);
    let request = pending
        .iter()
        .find(|view| view.device_id == device)
        .unwrap();
    request.id.to_string()
}

fn approve(id: &str, always: bool) -> ApprovalsCommand {
    ApprovalsCommand::Approve(ApproveArgs {
        id: id.into(),
        always,
    })
}

fn deny(id: &str) -> ApprovalsCommand {
    ApprovalsCommand::Deny(DenyArgs { id: id.into() })
}

#[test]
fn perm_task_cli_approvals_roundtrip() {
    let service = ApprovalService::new(Arc::new(CliOnly));
    let mut first = ask(&service, "laptop", "ls -la");

    // The list as D.2.4 defines it.
    let entry = listed(&service)[0].clone();
    let mut keys: Vec<&str> = entry
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "account_label",
            "capability",
            "command",
            "device_id",
            "device_name",
            "fingerprint",
            "id",
            "relation",
            "requested_at",
            "room",
            "surface"
        ]
    );
    assert_eq!(entry["capability"], "exec_commands");
    assert_eq!(entry["command"], json!({"mode": "shell", "text": "ls -la"}));
    assert_eq!(
        (entry["relation"].clone(), entry["surface"].clone()),
        (json!("direct"), json!("cli_only"))
    );
    let id = entry["id"].as_str().unwrap().to_string();
    let text = run(&service, ApprovalsCommand::Approvals, false).unwrap();
    assert!(
        text.contains(&format!("approval\t{id}\tdevice=Name laptop\t")),
        "{text}"
    );
    assert!(
        text.contains("\tcapability=commands\taccount=alice\tcommand=shell: ls -la\t"),
        "{text}"
    );
    assert!(
        text.contains("se share exec approve ID [--always]"),
        "{text}"
    );

    // „Diesmal erlauben“ with the first characters of the id.
    let answer = run(&service, approve(&id[..8], false), true).unwrap();
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        answer,
        json!({"id": id, "decision": "once", "decided": true})
    );
    assert_eq!(outcome(&mut first), ApprovalOutcome::Once);
    assert!(ApprovalService::pending(&service).is_empty());

    // „Immer erlauben“ and „Ablehnen“ answer exactly their own request.
    let mut second = ask(&service, "phone", "uptime");
    let mut third = ask(&service, "tablet", "df");
    let (phone, tablet) = (id_of(&service, "phone"), id_of(&service, "tablet"));
    let always = run(&service, approve(&phone, true), false).unwrap();
    assert_eq!(
        always.trim_end(),
        format!("approval\t{phone}\tdecision=always\tdecided=true")
    );
    assert_eq!(ApprovalService::pending(&service).len(), 1);
    run(&service, deny(&tablet), false).unwrap();
    assert_eq!(outcome(&mut second), ApprovalOutcome::Always);
    assert_eq!(outcome(&mut third), ApprovalOutcome::Denied);

    // An answered or malformed id is refused with the next step.
    for refused in [deny(&id), approve("xyz", false), approve(&id[..3], false)] {
        let error = run(&service, refused, false).unwrap_err();
        assert!(error.contains("se share exec approvals"), "{error}");
    }
    let empty = run(&service, ApprovalsCommand::Approvals, false).unwrap();
    assert!(empty.starts_with("approvals\t0\t"), "{empty}");
    assert_eq!(listed(&service), json!([]));
}

fn words(arguments: &[&str]) -> Vec<String> {
    arguments.iter().map(|word| word.to_string()).collect()
}

#[test]
fn perm_task_mcp_blocks_approve_and_setup() {
    let blocked: [&[&str]; 8] = [
        &["share", "exec", "approve", "0123abcd"],
        &["share", "exec", "approve", "0123abcd", "--always"],
        &["share", "exec", "--json", "deny", "0123abcd"],
        &["share", "exec", "accounts", "setup", "--remote-account"],
        &["share", "exec", "accounts", "allow", "deploy"],
        &["share", "exec", "accounts", "disallow", "deploy"],
        &["share", "exec", "accounts", "update"],
        &[
            "share",
            "exec",
            "accounts",
            "remove",
            "--delete-remote-account",
        ],
    ];
    for arguments in blocked {
        let refusal = mcp_refusal(&words(arguments))
            .unwrap_or_else(|| panic!("the MCP tool would run {arguments:?}"));
        assert!(refusal.starts_with("Diese Entscheidung trifft nur ein Mensch am Gerät."));
        assert!(
            refusal.contains(&format!("`se {}`", arguments.join(" "))),
            "{refusal}"
        );
    }
}

#[test]
fn perm_task_mcp_reads_approvals() {
    let open: [&[&str]; 7] = [
        &["share", "exec", "approvals", "--json"],
        &["share", "exec", "approvals"],
        &["share", "exec", "accounts", "--json"],
        &["share", "exec", "log", "--json"],
        &["share", "grants", "access", "--json"],
        &["share", "exec", "approve", "--help"],
        &["exec", "Laptop", "--", "approve"],
    ];
    for arguments in open {
        assert_eq!(mcp_refusal(&words(arguments)), None, "{arguments:?}");
    }
    // What an agent reads from `se share exec approvals --json`.
    let service = ApprovalService::new(Arc::new(CliOnly));
    let _waiting = ask(&service, "laptop", "make test");
    let pending = listed(&service);
    assert_eq!(pending.as_array().map(Vec::len), Some(1));
    let id = pending[0]["id"].as_str().unwrap();
    assert!(id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(pending[0]["command"]["text"], "make test");
    assert_eq!(pending[0]["device_name"], "Name laptop");
}
