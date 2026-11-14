//! `also_answered` with a number other than 0 (E28, E59; TODO
//! `SEC-ZUGANG-RUECKFRAGEN`): „Immer erlauben“ and „Ablehnen“ name in text and
//! JSON how many other open requests equal to theirs the worker answered too.
use std::sync::Arc;

use serde_json::Value;

use super::{run_with, ApprovalSource, ApprovalsCommand, ApproveArgs, DenyArgs};
use crate::share::access_approval::{
    ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalService, ApprovalSummary,
    ApprovalTicket, ApprovalView, ConsentNotifier,
};
use crate::share::access_grant::Capability;
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::relation_proof::ProvenPrincipal;

/// How many equal requests the counting source reports as answered too.
const ALSO: usize = 9;

/// A host without a graphical session: only the CLI answers.
struct CliOnly;

impl ConsentNotifier for CliOnly {
    fn show(&self, _request: &ApprovalView) -> Option<ApprovalSurface> {
        Some(ApprovalSurface::CliOnly)
    }
}

/// The approval service of this process, answering like a worker that also
/// answered `ALSO` equal requests.
struct Counting(ApprovalService);

impl ApprovalSource for Counting {
    fn pending(&self) -> Result<Vec<ApprovalView>, String> {
        Ok(self.0.pending())
    }

    fn decide(&self, id: &str, decision: ApprovalDecision) -> Result<(), String> {
        self.0.decide(&ApprovalId::parse(id)?, decision).map(|_| ())
    }

    fn decide_counted(&self, id: &str, decision: ApprovalDecision) -> Result<usize, String> {
        self.decide(id, decision).map(|()| ALSO)
    }
}

fn ask(service: &ApprovalService, device: &str) -> ApprovalTicket {
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
            text: "uptime".into(),
        }),
        policy_revision: 3,
        ..ApprovalSummary::default()
    };
    service.request(&principal, Capability::ExecCommands, summary)
}

fn id_of(source: &Counting, device: &str) -> String {
    let pending = source.0.pending();
    let request = pending
        .iter()
        .find(|view| view.device_id == device)
        .unwrap();
    request.id.to_string()
}

fn run(source: &Counting, command: ApprovalsCommand, json: bool) -> String {
    let mut out = Vec::new();
    run_with(source, command, json, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// Answers one request in text and one in JSON with `command` and checks the
/// count in both outputs.
fn answers_name_the_count(command: impl Fn(&str) -> ApprovalsCommand, key: &str) {
    let source = Counting(ApprovalService::new(Arc::new(CliOnly)));
    let _text_ticket = ask(&source.0, "phone");
    let _json_ticket = ask(&source.0, "tablet");
    let (phone, tablet) = (id_of(&source, "phone"), id_of(&source, "tablet"));

    let text = run(&source, command(&phone), false);
    let last = text.lines().last().unwrap_or_default();
    assert_eq!(
        last,
        format!("approval\t{phone}\tdecision={key}\tdecided=true\talso_answered={ALSO}"),
        "{text}"
    );

    let json: Value = serde_json::from_str(&run(&source, command(&tablet), true)).unwrap();
    assert_eq!(json["id"], tablet.as_str(), "{json}");
    assert_eq!(json["decision"], key, "{json}");
    assert_eq!(json["also_answered"], ALSO, "{json}");
    assert!(source.0.pending().is_empty());
}

#[test]
fn rueckfragen_task_cli_always_names_the_answered_count() {
    answers_name_the_count(
        |id| {
            ApprovalsCommand::Approve(ApproveArgs {
                id: id.into(),
                always: true,
            })
        },
        "always",
    );
}

#[test]
fn rueckfragen_task_cli_deny_names_the_answered_count() {
    answers_name_the_count(
        |id| ApprovalsCommand::Deny(DenyArgs { id: id.into() }),
        "deny",
    );
}
