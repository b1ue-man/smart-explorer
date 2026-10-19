//! PRM-M10, bridge part (strang-rechte.md R 401): `share.setAccess` keeps
//! what it does not name, `share.status` names the one account Android runs
//! commands as, a new program list keeps the account profile (B16), and a
//! confirmation goes from the app's notifier through the list to exactly one
//! answer. No worker runs here.
use std::sync::Arc;

use serde_json::json;

use super::super::{keep_account, program_settings, provider_json, targets_json};
use super::{
    access_change, activity, approvals_json, decision_args, AppConsent, OTHER_ACCOUNTS_NOTE,
};
use crate::share::access_approval::{
    ApprovalCommand, ApprovalDecision, ApprovalOutcome, ApprovalService, ApprovalSummary,
    ApprovalTicket,
};
use crate::share::access_grant::{
    AccessGrant, AccessGrantChange, AccountProfile, Capability, CapabilityDecision, Expiry,
};
use crate::share::exec_access_view::ApprovalSurface;
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::{
    DirectGrant, DirectGrantState, ExecGrant, ExecProviderStatus, ExecRunAs, ShareProfiles,
};

const NOW: i64 = 1_760_000_000;
/// An end long after any test run (2096).
const LATER: i64 = 4_000_000_000;

fn provider(label: &str) -> ExecProviderStatus {
    ExecProviderStatus {
        available: true,
        provider: "android-subreaper".into(),
        detail: String::new(),
        elevated: false,
        user_label: label.into(),
    }
}

fn change(
    account: Option<AccountProfile>,
    decisions: &[(Capability, CapabilityDecision)],
) -> AccessGrantChange {
    AccessGrantChange {
        account,
        capabilities: decisions.iter().copied().collect(),
        ..AccessGrantChange::default()
    }
}

#[test]
fn perm_task_bridge_set_access_absent_keeps() {
    // Only the terminal is named: account, commands, list and end stay.
    let terminal = access_change(
        &json!({ "targetKey": "direct/d/fp", "capabilities": { "execTerminal": "ask" } }),
        NOW,
    )
    .expect("terminal only");
    assert_eq!(
        terminal,
        change(None, &[(Capability::ExecTerminal, CapabilityDecision::Ask)])
    );
    let mut grant = AccessGrant::new(AccountProfile::Own)
        .with(Capability::ExecCommands, CapabilityDecision::Allow)
        .with(Capability::ExecTerminal, CapabilityDecision::Allow);
    grant.valid_until = Some(LATER);
    let mut programs = Vec::new();
    grant.apply(&terminal, &mut programs).expect("applied");
    assert_eq!(grant.account, AccountProfile::Own);
    assert_eq!(
        grant.decision(Capability::ExecCommands),
        CapabilityDecision::Allow
    );
    assert_eq!(
        grant.decision(Capability::ExecTerminal),
        CapabilityDecision::Ask
    );
    assert_eq!(grant.valid_until, Some(LATER));
    assert!(programs.is_empty());

    // `null` keeps account, capabilities and list ...
    let end = access_change(
        &json!({ "account": null, "capabilities": null, "programs": null, "validUntil": NOW + 60 }),
        NOW,
    )
    .expect("end only");
    assert_eq!(end.account, None);
    assert!(end.capabilities.is_empty());
    assert_eq!(end.programs, None);
    assert_eq!(end.expiry, Some(Expiry::At(NOW + 60)));
    // ... but removes the end, while a missing `validUntil` keeps it.
    let open = access_change(&json!({ "validUntil": null }), NOW).expect("no end");
    assert_eq!(open.expiry, Some(Expiry::None));
    grant.apply(&open, &mut programs).expect("applied");
    assert_eq!(grant.valid_until, None);
    assert_eq!(
        grant.decision(Capability::ExecTerminal),
        CapabilityDecision::Ask
    );

    // Nothing named is no change (no new revision for it).
    assert_eq!(
        access_change(&json!({ "targetKey": "direct/d/fp" }), NOW)
            .expect_err("empty")
            .kind,
        "invalid"
    );
    assert_eq!(
        access_change(&json!({ "account": { "kind": "own" } }), NOW)
            .expect("own")
            .account,
        Some(AccountProfile::Own)
    );
    assert_eq!(
        access_change(&json!({ "programs": [" uptime "] }), NOW)
            .expect("list")
            .programs,
        Some(vec!["uptime".to_string()])
    );
    for (refused, kind) in [
        (json!({ "account": { "kind": "remote" } }), "unsupported"),
        (json!({ "account": "own" }), "invalid"),
        (
            json!({ "capabilities": { "execCommands": "maybe" } }),
            "invalid",
        ),
        (json!({ "capabilities": { "bogus": "allow" } }), "invalid"),
        (json!({ "capabilities": ["execCommands"] }), "invalid"),
        (json!({ "programs": ["bin/tool"] }), "invalid"),
        (json!({ "validUntil": NOW }), "invalid"),
        (json!({ "validUntil": "morgen" }), "invalid"),
    ] {
        assert_eq!(
            access_change(&refused, NOW).expect_err("refused").kind,
            kind,
            "{refused}"
        );
    }
}

#[test]
fn perm_task_bridge_status_account_choices_android() {
    let status = provider_json(&provider("u0_a123"));
    // The fields before PRM-M10 stay.
    assert_eq!(status["available"], true);
    assert_eq!(status["provider"], "android-subreaper");
    assert_eq!(status["userLabel"], "u0_a123");
    // Only the app's own account, as `se share exec accounts --json` names it.
    assert_eq!(
        status["accountChoices"],
        json!([{
            "profile": "own",
            "label": "u0_a123",
            "available": true,
            "elevated": false,
            "adminGroup": false,
        }])
    );
    assert_eq!(status["elevated"], false);
    let note = status["otherAccountsNote"].as_str().expect("note");
    assert_eq!(note, OTHER_ACCOUNTS_NOTE);
    assert!(note.starts_with("App-Konto von Smart Explorer."));
    for part in [
        "Google-Drive-Tokens",
        "fremden Nutzerkennung",
        "Root (über su)",
        "Shizuku",
        "Arbeitsprofil oder privater Bereich",
    ] {
        assert!(note.contains(part), "{part}");
    }
    // The dialog rows: commands, terminal and, with SCR-M8, the screen.
    let rows: Vec<&str> = status["capabilities"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["key"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        rows,
        [
            "execCommands",
            "execTerminal",
            "screenView",
            "screenControl"
        ]
    );

    // Each target carries its access right.
    let mut exec = ExecGrant::default();
    exec.apply_access_change(
        &AccessGrantChange {
            expiry: Some(Expiry::At(LATER)),
            ..change(
                None,
                &[
                    (Capability::ExecCommands, CapabilityDecision::Allow),
                    (Capability::ExecTerminal, CapabilityDecision::Ask),
                ],
            )
        },
        NOW,
    )
    .expect("grant");
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: "d-laptop".into(),
        device_name: "Laptop".into(),
        public_key: "key".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec,
        write: false,
    });
    let targets = targets_json(&profiles);
    let access = &targets[0]["access"];
    assert_eq!(targets[0]["enabled"], true);
    assert_eq!(targets[0]["runAs"], "standard");
    assert_eq!(access["account"], json!({ "kind": "own" }));
    assert!(access["accountLabel"].is_string());
    assert_eq!(
        access["capabilities"],
        json!({ "execCommands": "allow", "execTerminal": "ask" })
    );
    assert_eq!(access["validUntil"], LATER);
    assert_eq!(access["expired"], false);
}

#[test]
fn perm_task_bridge_program_list_keeps_profile() {
    for profile in [
        AccountProfile::Own,
        AccountProfile::Remote,
        AccountProfile::Account {
            name: "build".into(),
        },
    ] {
        let mut grant = ExecGrant::default();
        grant
            .apply_access_change(
                &change(
                    Some(profile.clone()),
                    &[(Capability::ExecCommands, CapabilityDecision::Allow)],
                ),
                NOW,
            )
            .expect("grant");
        assert_eq!(grant.access_grant().account, profile);
        // `share.setExec {programs}` as the bridge sends it now.
        let settings = keep_account(
            program_settings(&json!({ "programs": ["uptime"] })).expect("list"),
            &grant.run_as,
        );
        grant
            .set_runtime_policy(true, settings.as_ref(), NOW + 1)
            .expect("list set");
        assert_eq!(grant.access_grant().account, profile, "{profile:?}");
        assert_eq!(grant.programs, ["uptime"]);
        assert_eq!(
            grant.access_grant().decision(Capability::ExecTerminal),
            CapabilityDecision::Off
        );
    }
    // The checked list alone names the Standardnutzer; the grant's account wins.
    let plain = program_settings(&json!({ "programs": [] }))
        .expect("list")
        .expect("given");
    assert_eq!(plain.run_as, ExecRunAs::Standard);
    let remote = ExecRunAs::Account {
        name: "se-remote".into(),
    };
    let kept = keep_account(Some(plain), &remote).expect("kept");
    assert_eq!(kept.run_as, remote);
    assert!(kept.programs.is_empty());
    assert_eq!(keep_account(None, &remote), None);
}

fn laptop() -> ProvenPrincipal {
    ProvenPrincipal {
        relation_kind: "direct".into(),
        relation_id: "d-laptop".into(),
        device_id: "d-laptop".into(),
        device_name: "Laptop".into(),
        public_key: "key".into(),
        fingerprint: "fp-laptop".into(),
        node_id: "node".into(),
    }
}

fn outcome(ticket: &mut ApprovalTicket) -> ApprovalOutcome {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(ticket.outcome())
}

#[test]
fn perm_task_bridge_approvals_roundtrip() {
    let service = ApprovalService::new(Arc::new(AppConsent));
    let summary = ApprovalSummary {
        account_label: Some("u0_a123".into()),
        command: Some(ApprovalCommand::Shell {
            text: "uptime".into(),
        }),
        policy_revision: 3,
        ..ApprovalSummary::default()
    };
    let before = activity();
    let mut ticket = service.request(&laptop(), Capability::ExecCommands, summary.clone());
    // The app's notifier takes it and wakes the poller (its `share` event).
    assert_eq!(ticket.surface(), Some(ApprovalSurface::Notification));
    assert!(activity() > before, "an open request wakes the app");

    let listed = approvals_json(&service.pending());
    let first = &listed["pending"][0];
    assert_eq!(first["id"], ticket.id().as_str());
    assert_eq!(first["deviceName"], "Laptop");
    assert_eq!(first["deviceId"], "d-laptop");
    assert_eq!(first["fingerprint"], "fp-laptop");
    assert_eq!(first["relation"], "direct");
    assert_eq!(first["capability"], "execCommands");
    assert_eq!(first["capabilityLabel"], "Befehle");
    assert_eq!(first["text"], "Laptop möchte Befehle ausführen");
    assert_eq!(first["accountLabel"], "u0_a123");
    assert_eq!(
        first["command"],
        json!({ "mode": "shell", "text": "uptime" })
    );
    assert_eq!(first["surface"], "notification");
    assert_eq!(first["olderClientTerminal"], false);

    // The answer names exactly this request and counts once.
    let (id, decision) = decision_args(&json!({
        "approvalId": first["id"].clone(),
        "decision": "once",
    }))
    .expect("answer");
    assert_eq!(&id, ticket.id());
    let shown = activity();
    service.decide(&id, decision).expect("decided");
    assert_eq!(outcome(&mut ticket), ApprovalOutcome::Once);
    assert!(activity() > shown, "an ended request wakes the app");
    assert!(service.pending().is_empty());
    assert!(service.decide(&id, ApprovalDecision::Deny).is_err());

    // A request the app can show nowhere ends refused, never allowed.
    let mut hidden = service.request(&laptop(), Capability::ExecTerminal, summary);
    let hidden_id = hidden.id().clone();
    service.report_undeliverable(&hidden_id).expect("reported");
    assert_eq!(outcome(&mut hidden), ApprovalOutcome::Undeliverable);

    // Malformed answers are refused before the worker is asked.
    let some_id = "ab".repeat(16);
    for refused in [
        json!({ "approvalId": "x", "decision": "once" }),
        json!({ "approvalId": some_id, "decision": "vielleicht" }),
        json!({ "approvalId": some_id }),
        json!({ "decision": "deny" }),
    ] {
        assert_eq!(
            decision_args(&refused).expect_err("refused").kind,
            "invalid",
            "{refused}"
        );
    }
    for (word, expected) in [
        ("once", ApprovalDecision::Once),
        ("always", ApprovalDecision::Always),
        ("deny", ApprovalDecision::Deny),
    ] {
        let (_, decision) =
            decision_args(&json!({ "approvalId": "AB".repeat(16), "decision": word }))
                .expect("valid");
        assert_eq!(decision, expected);
    }
}
