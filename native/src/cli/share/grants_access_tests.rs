//! Acceptance of `se share grants access` and its aliases (PRM-M8, D.2.4):
//! the JSON and text shape, `enable`/`disable` unchanged, and the room
//! change as a snapshot of the admitted members.
use std::collections::BTreeMap;

use clap::{CommandFactory, Parser};
use serde_json::{json, Value};

use super::super::grants_access_output as output;
use super::super::grants_exec::{room_targets, toggle_request, AccessTarget, ExecGrantCommand};
use super::{request, AccessArgs};
use crate::share::access_grant::{
    AccessGrantView, AccountProfile, Capability, CapabilityDecision, Expiry,
};
use crate::share::{
    DirectGrant, DirectGrantState, ExecGrant, ExecGrantSettings, ExecGrantTarget, ExecRunAs,
    RoomMember, RoomMemberAdmission, RoomMemberFlags, RoomPolicy, RoomProfile, ShareProfiles,
    ShareStatus,
};

const NOW: i64 = 1_760_000_000;

#[derive(Parser)]
struct AccessCli {
    #[command(flatten)]
    args: AccessArgs,
}

#[derive(Parser)]
struct ExecCli {
    #[command(subcommand)]
    command: ExecGrantCommand,
}

fn access_args(words: &[&str]) -> AccessArgs {
    let words = std::iter::once("access").chain(words.iter().copied());
    AccessCli::try_parse_from(words).unwrap().args
}

fn exec_command(words: &[&str]) -> ExecGrantCommand {
    let words = std::iter::once("exec").chain(words.iter().copied());
    ExecCli::try_parse_from(words).unwrap().command
}

fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn direct(device: &str, exec: ExecGrant) -> DirectGrant {
    DirectGrant {
        device_id: device.into(),
        device_name: format!("Name {device}"),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("key-{device}"),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec,
        write: false,
    }
}

fn member(
    device: &str,
    admission: RoomMemberAdmission,
    blocked: bool,
    exec: ExecGrant,
) -> RoomMember {
    RoomMember {
        device_id: device.into(),
        device_name: device.into(),
        fingerprint: format!("fp-{device}"),
        public_key: format!("key-{device}"),
        node_id: format!("key-{device}"),
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status: ShareStatus::Waiting,
        blocked,
        exec,
        presence: None,
        relation: RoomMemberFlags {
            admission,
            ..RoomMemberFlags::default()
        },
    }
}

fn allowed() -> ExecGrant {
    let mut grant = ExecGrant::default();
    grant.set_runtime_policy(true, None, NOW).unwrap();
    grant
}

#[test]
fn perm_task_cli_grants_access_json_shape() {
    // One command sets account, both capabilities and an end of 8 hours.
    let args = access_args(&[
        "Laptop",
        "--as",
        "remote",
        "--commands",
        "allow",
        "--terminal",
        "ask",
        "--for",
        "8h",
    ]);
    let wanted = request(&args, NOW).unwrap().unwrap();
    let mut exec = ExecGrant::default();
    let change = wanted.change_for(&exec);
    assert_eq!(change.account, Some(AccountProfile::Remote));
    assert_eq!(change.expiry, Some(Expiry::At(NOW + 8 * 3_600)));
    exec.apply_access_change(&change, NOW).unwrap();
    let target = AccessTarget {
        target: ExecGrantTarget::Direct {
            device_id: "laptop".into(),
            public_key: "key-laptop".into(),
            fingerprint: "fp-laptop".into(),
            node_id: "key-laptop".into(),
        },
        label: "Laptop".into(),
        device_id: "laptop".into(),
        fingerprint: "fp-laptop".into(),
        room: None,
        exec: exec.clone(),
    };
    let access = json!({
        "account": {"kind": "remote"},
        "account_label": "se-remote",
        "capabilities": {"exec_commands": "allow", "exec_terminal": "ask"},
        "valid_until": NOW + 8 * 3_600,
        "expired": false,
        // V2 grant fields of the view since c5d3e899 (`AccessGrantView`):
        // a V1 grant without constraints.
        "version": 1,
        "constraints": {},
        "legacy_reader_disables_grant": false,
    });
    let shown = output::target_value(&target, "alice", NOW);
    assert_eq!(
        sorted_keys(&shown),
        [
            "access",
            "device_id",
            "device_name",
            "fingerprint",
            "policy_revision",
            "programs",
            "relation",
            "room",
            "target"
        ]
    );
    assert_eq!(shown["access"], access);
    assert_eq!(shown["target"], "exec://direct/laptop/fp-laptop");
    assert_eq!(
        (shown["relation"].clone(), shown["room"].clone()),
        (json!("direct"), Value::Null)
    );
    let view = AccessGrantView::new(&exec.access_grant(), "alice", NOW);
    let result = json!({"persisted": true, "applied": true, "revision": 2});
    let changed = output::change_value(&target, result, Some(&view));
    assert_eq!(
        sorted_keys(&changed),
        [
            "access",
            "device_id",
            "device_name",
            "fingerprint",
            "relation",
            "result",
            "room",
            "target"
        ]
    );
    assert_eq!(changed["access"], access);
    let line = output::change_line(&target, &changed["result"], Some(&view));
    assert!(line.contains("\tpersisted=true\tapplied=true\trevision=2\taccount=se-remote\tcommands=allow\tterminal=ask\tuntil="));

    // `se share grants exec --json`: every key of 0.5.182 plus `access`.
    let choice = output::exec_choice_value(&target, "alice", NOW);
    assert_eq!(
        sorted_keys(&choice),
        [
            "access",
            "device_id",
            "device_name",
            "enabled",
            "fingerprint",
            "programs",
            "run_as",
            "standard_user",
            "target"
        ]
    );
    assert_eq!(choice["access"], access);

    // `se share grants --json`: `exec` keeps its keys and gains `access`; a
    // grant whose end has passed reads as disabled (the clock is now).
    let profiles = ShareProfiles {
        direct_grants: vec![direct("laptop", exec)],
        ..ShareProfiles::default()
    };
    let listed = &super::super::values(&profiles)[0]["exec"];
    assert_eq!(
        sorted_keys(listed),
        [
            "access",
            "changed_at",
            "enabled",
            "policy_revision",
            "programs",
            "run_as"
        ]
    );
    assert_eq!(listed["access"]["capabilities"], access["capabilities"]);
    assert_eq!(
        (
            listed["enabled"].clone(),
            listed["access"]["expired"].clone()
        ),
        (json!(false), json!(true))
    );
    let text = &super::super::text(&profiles)[0];
    assert!(text.contains("\texec=disabled\t"), "{text}");
    assert!(
        text.contains("\texec_as=se-remote\texec_commands=allow\texec_terminal=ask\texec_until=")
    );
    assert!(
        text.ends_with("\texec_run_as=se-remote\texec_programs=full-shell"),
        "{text}"
    );

    // Time values: durations, RFC 3339 and the local time; never the past.
    assert_eq!(output::parse_duration("1d12h"), Ok(129_600));
    assert_eq!(output::parse_duration("30m"), Ok(1_800));
    for wrong in ["", "8", "8x", "h", "0m"] {
        assert!(output::parse_duration(wrong).is_err(), "{wrong:?}");
    }
    let at = output::parse_until("2025-10-10T09:46:40+02:00", NOW).unwrap();
    assert_eq!(at, 1_760_082_400);
    assert!(output::parse_until("2025-10-10T09:46", NOW).is_ok());
    assert!(output::parse_until("2020-01-01T00:00:00Z", NOW)
        .unwrap_err()
        .contains("--off"));
    assert_eq!(output::until_text(None), "none");
}

#[test]
fn perm_task_cli_enable_alias_unchanged() {
    // Every form and option of 0.5.182 still parses.
    for words in [
        vec!["se", "share", "grants", "exec"],
        vec!["se", "share", "grants", "exec", "--json"],
        vec!["se", "share", "grants", "exec", "enable", "--yes"],
        vec![
            "se",
            "share",
            "grants",
            "exec",
            "enable",
            "Laptop",
            "--run-as",
            "deploy",
            "--allow-program",
            "uptime",
            "--allow-program",
            "/usr/bin/df",
            "--yes",
        ],
        vec![
            "se",
            "share",
            "grants",
            "exec",
            "enable",
            "exec://direct/d/f",
            "--full",
        ],
        vec![
            "se", "share", "grants", "exec", "disable", "Laptop", "--yes",
        ],
        vec![
            "se",
            "share",
            "grants",
            "access",
            "Laptop",
            "--commands",
            "ask",
        ],
    ] {
        assert!(
            crate::cli::Cli::try_parse_from(words.clone()).is_ok(),
            "{words:?}"
        );
    }
    let both = [
        "se",
        "share",
        "grants",
        "exec",
        "enable",
        "--full",
        "--allow-program",
        "x",
    ];
    assert!(crate::cli::Cli::try_parse_from(both).is_err());

    // The request to the worker is `MutateExecGrant` as before.
    let listed = ExecGrant {
        enabled: true,
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        programs: vec!["uptime".into()],
        ..ExecGrant::default()
    };
    assert_eq!(
        toggle_request(&exec_command(&["enable"]), &listed),
        (true, None)
    );
    assert_eq!(
        toggle_request(&exec_command(&["disable", "--yes"]), &listed),
        (false, None)
    );
    let full = ExecGrantSettings {
        run_as: listed.run_as.clone(),
        programs: Vec::new(),
    };
    let request = toggle_request(&exec_command(&["enable", "--full"]), &listed);
    assert_eq!(request, (true, Some(full)));

    // The worker maps it as before (D.2.3): disable turns commands and
    // terminal off and keeps account and list; enable allows commands, and
    // the terminal only without a list.
    let decision = |grant: &ExecGrant, capability| grant.access_grant().decision(capability);
    let mut grant = listed.clone();
    grant.set_runtime_policy(false, None, NOW).unwrap();
    assert_eq!(
        decision(&grant, Capability::ExecCommands),
        CapabilityDecision::Off
    );
    assert_eq!(grant.programs, ["uptime"]);
    assert_eq!(grant.run_as, listed.run_as);
    grant.set_runtime_policy(true, None, NOW + 1).unwrap();
    assert_eq!(
        decision(&grant, Capability::ExecCommands),
        CapabilityDecision::Allow
    );
    assert_eq!(
        decision(&grant, Capability::ExecTerminal),
        CapabilityDecision::Off
    );
    let open = allowed();
    assert_eq!(
        decision(&open, Capability::ExecTerminal),
        CapabilityDecision::Allow
    );
    assert_eq!(open.access_grant().account, AccountProfile::Own);

    // The confirmation flag names the access warning (C6).
    let mut command = crate::cli::Cli::command();
    for name in ["share", "grants", "exec", "enable"] {
        command = command.find_subcommand(name).unwrap().clone();
    }
    let help = command.render_long_help().to_string();
    assert!(
        help.contains("Confirm the access warning without a prompt"),
        "{help}"
    );
    assert!(help.contains("se share grants access"), "{help}");
}

#[test]
fn perm_task_cli_room_all_members_snapshot() {
    let none = ExecGrant::default;
    let ops = RoomProfile {
        id: "room-profile".into(),
        name: "Ops".into(),
        room_id: "room-wire".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![
            member("a", RoomMemberAdmission::Admitted, false, allowed()),
            member("b", RoomMemberAdmission::Admitted, false, none()),
            member("c", RoomMemberAdmission::Pending, true, none()),
            member("d", RoomMemberAdmission::Admitted, true, none()),
        ],
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    };
    let mut profiles = ShareProfiles {
        rooms: vec![ops],
        ..ShareProfiles::default()
    };
    // Only the admitted members, each with its own pinned identity.
    let (room, members) = room_targets(&profiles, "Ops").unwrap();
    assert_eq!(
        (room.id.as_str(), room.room_id.as_str()),
        ("room-profile", "room-wire")
    );
    let labels: Vec<&str> = members.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["Ops / a", "Ops / b"]);
    assert_eq!(
        members[1].target,
        ExecGrantTarget::RoomMember {
            room_id: "room-wire".into(),
            device_id: "b".into(),
            public_key: "key-b".into(),
            fingerprint: "fp-b".into(),
            node_id: "key-b".into(),
        }
    );
    for selector in ["room-profile", "room-wire"] {
        assert_eq!(room_targets(&profiles, selector).unwrap().1.len(), 2);
    }

    // One command, one change per member, each from that member's grant.
    let args = access_args(&[
        "--room",
        "Ops",
        "--all-members",
        "--commands",
        "allow",
        "--terminal",
        "ask",
    ]);
    let wanted = request(&args, NOW).unwrap().unwrap();
    let changes: Vec<_> = members.iter().map(|m| wanted.change_for(&m.exec)).collect();
    assert_eq!(changes[0], changes[1]);
    let ask = request(
        &access_args(&["--room", "Ops", "--all-members", "--ask"]),
        NOW,
    )
    .unwrap()
    .unwrap();
    let asking = BTreeMap::from([
        (Capability::ExecCommands, CapabilityDecision::Ask),
        (Capability::ExecTerminal, CapabilityDecision::Ask),
    ]);
    assert_eq!(ask.change_for(&members[0].exec).capabilities, asking);
    assert!(ask.change_for(&members[1].exec).is_empty());

    // A snapshot: a member admitted later gets nothing from this command.
    let late = member("e", RoomMemberAdmission::Admitted, false, none());
    profiles.rooms[0].members.push(late);
    assert_eq!(members.len(), 2);
    assert_eq!(room_targets(&profiles, "Ops").unwrap().1.len(), 3);

    // A left room is refused with the next step; --room needs --all-members
    // and excludes a device selector.
    profiles.rooms[0].auto_join = false;
    let left = room_targets(&profiles, "Ops").unwrap_err();
    assert!(left.contains("se share room rejoin room-profile"), "{left}");
    assert!(AccessCli::try_parse_from(["access", "--room", "Ops"]).is_err());
    let both = ["access", "Laptop", "--room", "Ops", "--all-members"];
    assert!(AccessCli::try_parse_from(both).is_err());
}
