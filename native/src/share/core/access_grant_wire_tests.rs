//! Stored form of access rights (D.2.1, C1): grants of 0.5.180 to 0.5.182
//! port without a change of meaning and are written back byte for byte,
//! older readers see every new grant switched off, and an unknown version
//! fails closed and survives a save.
use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::share::access_grant::{
    AccessGrantChange, AccountProfile, Capability, CapabilityDecision, Expiry,
};
use crate::share::{ExecGrant, ExecRunAs};

/// The fields of `ExecGrant` that 0.5.180 reads.
#[derive(Deserialize)]
struct Reader0180 {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    policy_revision: u64,
}

/// The fields of `ExecGrant` that 0.5.181 and 0.5.182 read, and their rule.
#[derive(Deserialize)]
struct Reader0182 {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    run_as: Value,
    #[serde(default)]
    programs: Vec<String>,
    #[serde(default)]
    restricted_enabled: bool,
}

impl Reader0182 {
    fn switched_on(&self) -> bool {
        let restricted = !self.programs.is_empty()
            || self.run_as.get("kind").and_then(Value::as_str) == Some("account");
        self.enabled || (self.restricted_enabled && restricted)
    }
}

const ALLOW: CapabilityDecision = CapabilityDecision::Allow;
const ASK: CapabilityDecision = CapabilityDecision::Ask;
const OFF: CapabilityDecision = CapabilityDecision::Off;

fn read(text: &str) -> ExecGrant {
    serde_json::from_str(text).unwrap()
}

fn written(grant: &ExecGrant) -> String {
    serde_json::to_string(grant).unwrap()
}

fn decisions(grant: &ExecGrant) -> [CapabilityDecision; 4] {
    let access = grant.access_grant();
    Capability::ALL.map(|capability| access.decision(capability))
}

fn change(capabilities: &[(Capability, CapabilityDecision)]) -> AccessGrantChange {
    AccessGrantChange {
        capabilities: capabilities.iter().copied().collect::<BTreeMap<_, _>>(),
        ..AccessGrantChange::default()
    }
}

#[test]
fn perm_task_port_full_shell_0_5_180() {
    // Exactly the five fields 0.5.180 wrote.
    let stored = r#"{"enabled":true,"policy_revision":6,"changed_at":1700000000,"source_request_id":null,"source_decision_revision":2}"#;
    let grant = read(stored);
    assert!(grant.enabled);
    assert_eq!(grant.policy_revision, 6);
    let access = grant.access_grant();
    assert_eq!(access.account, AccountProfile::Own);
    assert_eq!(decisions(&grant), [ALLOW, ALLOW, OFF, OFF]);
    assert_eq!(access.valid_until, None);
    // Written as 0.5.181 and 0.5.182 write it: the old fields and `run_as`.
    assert_eq!(
        written(&grant),
        r#"{"enabled":true,"policy_revision":6,"changed_at":1700000000,"source_request_id":null,"source_decision_revision":2,"run_as":{"kind":"standard"}}"#
    );
    let head: Reader0180 = serde_json::from_str(&written(&grant)).unwrap();
    assert!(head.enabled);
    assert_eq!(head.policy_revision, 6);
}

#[test]
fn perm_task_port_program_list_0_5_182() {
    let stored = r#"{"enabled":false,"policy_revision":4,"changed_at":9,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"standard"},"programs":["/usr/bin/uptime"],"restricted_enabled":true}"#;
    let grant = read(stored);
    assert!(grant.enabled);
    assert_eq!(grant.programs, ["/usr/bin/uptime"]);
    assert_eq!(grant.access_grant().account, AccountProfile::Own);
    // Commands of the listed program only; no terminal.
    assert_eq!(decisions(&grant), [ALLOW, OFF, OFF, OFF]);
    assert_eq!(written(&grant), stored);
}

#[test]
fn perm_task_port_run_as_root_worker() {
    let stored = r#"{"enabled":false,"policy_revision":9,"changed_at":3,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"account","name":"deploy"},"restricted_enabled":true}"#;
    let grant = read(stored);
    let access = grant.access_grant();
    assert_eq!(
        access.account,
        AccountProfile::Account {
            name: "deploy".into()
        }
    );
    assert_eq!(decisions(&grant), [ALLOW, ALLOW, OFF, OFF]);
    assert_eq!(
        grant.run_as,
        ExecRunAs::Account {
            name: "deploy".into()
        }
    );
    assert_eq!(written(&grant), stored);
}

#[test]
fn perm_task_port_disabled_keeps_settings() {
    let stored = r#"{"enabled":false,"policy_revision":5,"changed_at":8,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"account","name":"deploy"},"programs":["uptime"]}"#;
    let mut grant = read(stored);
    assert!(!grant.enabled);
    assert_eq!(decisions(&grant), [OFF, OFF, OFF, OFF]);
    assert_eq!(grant.programs, ["uptime"]);
    assert_eq!(
        grant.access_grant().account,
        AccountProfile::Account {
            name: "deploy".into()
        }
    );
    assert_eq!(written(&grant), stored);
    // Switching on again brings account and list back.
    grant.set_runtime_enabled(true, 9).unwrap();
    assert_eq!(decisions(&grant), [ALLOW, OFF, OFF, OFF]);
    assert_eq!(grant.programs, ["uptime"]);
    assert_eq!(
        grant.run_as,
        ExecRunAs::Account {
            name: "deploy".into()
        }
    );
}

#[test]
fn perm_task_write_legacy_form_bytes_unchanged() {
    // Every grant the old fields express exactly, made through the new
    // change, is written exactly as 0.5.182 writes it.
    let mut full = ExecGrant::default();
    full.apply_access_change(
        &change(&[
            (Capability::ExecCommands, ALLOW),
            (Capability::ExecTerminal, ALLOW),
        ]),
        10,
    )
    .unwrap();
    assert!(full.access.is_none());
    assert_eq!(
        written(&full),
        r#"{"enabled":true,"policy_revision":1,"changed_at":10,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"standard"}}"#
    );

    let mut listed = ExecGrant::default();
    let programs = AccessGrantChange {
        programs: Some(vec!["uptime".into()]),
        ..change(&[(Capability::ExecCommands, ALLOW)])
    };
    listed.apply_access_change(&programs, 11).unwrap();
    assert_eq!(
        written(&listed),
        r#"{"enabled":false,"policy_revision":1,"changed_at":11,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"standard"},"programs":["uptime"],"restricted_enabled":true}"#
    );

    let mut account = ExecGrant::default();
    let deploy = AccessGrantChange {
        account: Some(AccountProfile::Account {
            name: "deploy".into(),
        }),
        ..change(&[
            (Capability::ExecCommands, ALLOW),
            (Capability::ExecTerminal, ALLOW),
        ])
    };
    account.apply_access_change(&deploy, 12).unwrap();
    assert_eq!(
        written(&account),
        r#"{"enabled":false,"policy_revision":1,"changed_at":12,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"account","name":"deploy"},"restricted_enabled":true}"#
    );

    let mut off = account.clone();
    off.apply_access_change(
        &change(&[
            (Capability::ExecCommands, OFF),
            (Capability::ExecTerminal, OFF),
        ]),
        13,
    )
    .unwrap();
    assert_eq!(
        written(&off),
        r#"{"enabled":false,"policy_revision":2,"changed_at":13,"source_request_id":null,"source_decision_revision":null,"run_as":{"kind":"account","name":"deploy"}}"#
    );
    for grant in [&full, &listed, &account, &off] {
        assert_eq!(&read(&written(grant)), grant);
    }
}

#[test]
fn perm_task_old_reader_sees_new_grant_off() {
    let base = || {
        change(&[
            (Capability::ExecCommands, ALLOW),
            (Capability::ExecTerminal, ALLOW),
        ])
    };
    let cases = [
        // A capability on `ask`.
        change(&[
            (Capability::ExecCommands, ASK),
            (Capability::ExecTerminal, OFF),
        ]),
        // The remote-access account.
        AccessGrantChange {
            account: Some(AccountProfile::Remote),
            ..base()
        },
        // An end.
        AccessGrantChange {
            expiry: Some(Expiry::At(1_760_728_800)),
            ..base()
        },
        // Commands without a terminal and without a program list.
        change(&[
            (Capability::ExecCommands, ALLOW),
            (Capability::ExecTerminal, OFF),
        ]),
        // A screen capability alone.
        change(&[(Capability::ScreenView, ALLOW)]),
    ];
    for case in cases {
        let mut grant = ExecGrant::default();
        grant.apply_access_change(&case, 20).unwrap();
        let text = written(&grant);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["enabled"], false, "{text}");
        assert!(value.get("restricted_enabled").is_none(), "{text}");
        assert_eq!(value["access"]["version"], 1, "{text}");
        let head: Reader0180 = serde_json::from_str(&text).unwrap();
        assert!(!head.enabled, "0.5.180 must read {text} as off");
        let previous: Reader0182 = serde_json::from_str(&text).unwrap();
        assert!(!previous.switched_on(), "0.5.182 must read {text} as off");
        // This version reads its own output unchanged.
        assert_eq!(read(&text), grant, "{text}");
    }
    // The remote account is written for older readers as `se-remote`.
    let mut remote = ExecGrant::default();
    remote
        .apply_access_change(
            &AccessGrantChange {
                account: Some(AccountProfile::Remote),
                ..base()
            },
            21,
        )
        .unwrap();
    let value: Value = serde_json::from_str(&written(&remote)).unwrap();
    assert_eq!(
        value["run_as"],
        json!({"kind": "account", "name": "se-remote"})
    );
    assert_eq!(value["access"]["account"], json!({"kind": "remote"}));
}

#[test]
fn perm_task_unknown_access_version_fail_closed() {
    let object = json!({
        "version": 2,
        "account": {"kind": "own"},
        "capabilities": {"exec_commands": "allow", "exec_terminal": "allow", "files": "allow"},
        "approver": "someone"
    });
    let stored = json!({
        "enabled": false,
        "policy_revision": 3,
        "changed_at": 4,
        "source_request_id": null,
        "source_decision_revision": null,
        "run_as": {"kind": "standard"},
        "access": object.clone(),
    });
    let mut grant: ExecGrant = serde_json::from_value(stored).unwrap();
    assert!(!grant.enabled);
    assert_eq!(decisions(&grant), [OFF, OFF, OFF, OFF]);
    for capability in Capability::ALL {
        assert_eq!(grant.decision_at(capability, 5), OFF);
    }
    // Saving keeps the object exactly until the host changes the grant.
    let saved: Value = serde_json::to_value(&grant).unwrap();
    assert_eq!(saved["access"], object);
    assert_eq!(saved["enabled"], false);
    grant.set_runtime_enabled(true, 6).unwrap();
    assert_eq!(decisions(&grant), [ALLOW, ALLOW, OFF, OFF]);
    let changed: Value = serde_json::to_value(&grant).unwrap();
    assert!(changed.get("access").is_none());
    assert_eq!(changed["enabled"], true);

    // A version-1 object this version cannot read fails closed as well.
    let unreadable = json!({
        "enabled": false,
        "run_as": {"kind": "standard"},
        "access": {"version": 1, "account": {"kind": "own"}, "capabilities": {"exec_commands": "sometimes"}}
    });
    let grant: ExecGrant = serde_json::from_value(unreadable.clone()).unwrap();
    assert_eq!(decisions(&grant), [OFF, OFF, OFF, OFF]);
    assert_eq!(
        serde_json::to_value(&grant).unwrap()["access"],
        unreadable["access"]
    );

    // Capability keys and fields a version-1 object adds stay and never act.
    let newer = json!({
        "enabled": false,
        "run_as": {"kind": "standard"},
        "access": {
            "version": 1,
            "account": {"kind": "own"},
            "capabilities": {"exec_commands": "ask", "file_delete": "allow"},
            "note": "kept"
        }
    });
    let mut grant: ExecGrant = serde_json::from_value(newer).unwrap();
    assert_eq!(decisions(&grant), [ASK, OFF, OFF, OFF]);
    grant
        .apply_access_change(&change(&[(Capability::ExecTerminal, ALLOW)]), 7)
        .unwrap();
    let saved: Value = serde_json::to_value(&grant).unwrap();
    assert_eq!(saved["access"]["capabilities"]["file_delete"], "allow");
    assert_eq!(saved["access"]["capabilities"]["exec_terminal"], "allow");
    assert_eq!(saved["access"]["note"], "kept");
}
