//! Acceptance of the access completion (PRM-M8, D.2.4): the state label,
//! devices, rooms, accounts, request ids and the new options on the command
//! line.
use std::collections::BTreeMap;

use clap::CommandFactory;
use clap_complete::CompletionCandidate;

use super::{
    access_state_label, account_candidates_from, account_candidates_from_choices,
    approval_candidates_from, room_candidates_from, target_candidates,
};
use crate::share::access_approval::{ApprovalId, ApprovalView};
use crate::share::access_grant::{
    AccessGrantChange, AccountProfile, Capability, CapabilityDecision, Expiry,
};
use crate::share::{
    DirectGrant, DirectGrantState, ExecGrant, ExecRunAs, RoomMember, RoomMemberAdmission,
    RoomMemberFlags, RoomPolicy, RoomProfile, ShareProfiles, ShareStatus,
};

const NOW: i64 = 1_760_000_000;

fn values(candidates: &[CompletionCandidate]) -> Vec<String> {
    let value = |c: &CompletionCandidate| c.get_value().to_string_lossy().into_owned();
    candidates.iter().map(value).collect()
}

fn helps(candidates: &[CompletionCandidate]) -> Vec<String> {
    let help = |c: &CompletionCandidate| c.get_help().map(ToString::to_string).unwrap_or_default();
    candidates.iter().map(help).collect()
}

fn local_minutes(unix_secs: i64) -> String {
    let time = chrono::DateTime::<chrono::Utc>::from_timestamp(unix_secs, 0).unwrap();
    time.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn member(device: &str, admission: RoomMemberAdmission, exec: ExecGrant) -> RoomMember {
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
        blocked: admission == RoomMemberAdmission::Pending,
        exec,
        presence: None,
        relation: RoomMemberFlags {
            admission,
            ..RoomMemberFlags::default()
        },
    }
}

fn complete(words: &[&str]) -> Vec<String> {
    let args = words.iter().map(std::ffi::OsString::from).collect();
    let candidates = clap_complete::engine::complete(
        &mut crate::cli::Cli::command(),
        args,
        words.len() - 1,
        None,
    )
    .unwrap();
    values(&candidates)
}

#[test]
fn perm_task_completions_access() {
    // The state label of D.2.4.
    let mut remote = ExecGrant::default();
    assert_eq!(
        access_state_label(&remote, NOW),
        "disabled · commands off · terminal off"
    );
    let end = NOW + 8 * 3_600;
    let change = AccessGrantChange {
        account: Some(AccountProfile::Remote),
        capabilities: BTreeMap::from([
            (Capability::ExecCommands, CapabilityDecision::Allow),
            (Capability::ExecTerminal, CapabilityDecision::Ask),
        ]),
        programs: None,
        expiry: Some(Expiry::At(end)),
    };
    remote.apply_access_change(&change, NOW).unwrap();
    let state = "as se-remote · commands allow · terminal ask";
    let until = local_minutes(end);
    assert_eq!(
        access_state_label(&remote, NOW),
        format!("enabled · {state} · until {until}")
    );
    assert_eq!(
        access_state_label(&remote, end),
        format!("disabled · {state} · ended {until}")
    );
    let listed = ExecGrant {
        enabled: true,
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        programs: vec!["uptime".into(), "/usr/bin/df".into()],
        ..ExecGrant::default()
    };
    assert_eq!(
        access_state_label(&listed, NOW),
        "enabled · as deploy · commands allow · terminal off · programs uptime,/usr/bin/df"
    );

    // Devices, rooms and accounts of this host's profiles.
    let profiles = ShareProfiles {
        direct_grants: vec![DirectGrant {
            device_id: "laptop".into(),
            device_name: "Laptop".into(),
            public_key: "key-laptop".into(),
            fingerprint: "fp-laptop".into(),
            node_id: "key-laptop".into(),
            state: DirectGrantState::Accepted,
            updated_at: 1,
            exec: remote,
            write: false,
        }],
        rooms: vec![RoomProfile {
            id: "room-profile".into(),
            name: "Ops".into(),
            room_id: "room-wire".into(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: vec![
                member("a", RoomMemberAdmission::Admitted, listed),
                member("c", RoomMemberAdmission::Pending, ExecGrant::default()),
            ],
            exports: Default::default(),
            policy: RoomPolicy::new_room(),
        }],
        ..ShareProfiles::default()
    };
    let devices = target_candidates(&profiles, NOW);
    assert_eq!(
        values(&devices),
        [
            "exec://direct/laptop/fp-laptop",
            "exec://room/room-wire/a/fp-a"
        ]
    );
    assert_eq!(
        helps(&devices)[0],
        format!("Laptop · direct · enabled · {state} · until {until}")
    );
    assert!(helps(&devices)[1].starts_with("Ops / a · room · enabled · as deploy"));
    let rooms = room_candidates_from(&profiles);
    assert_eq!(values(&rooms), ["room-profile"]);
    assert_eq!(helps(&rooms), ["Ops · 1 admitted members"]);
    let accounts = account_candidates_from(&profiles);
    assert_eq!(values(&accounts), ["own", "remote", "account:deploy"]);

    // Open requests by id for `approve` and `deny`.
    let request = ApprovalView {
        id: ApprovalId::parse(&"ab".repeat(16)).unwrap(),
        device_name: "Laptop".into(),
        device_id: "laptop".into(),
        fingerprint: "fp-laptop".into(),
        relation: "direct".into(),
        relation_id: "direct-test".into(),
        room: None,
        capability: Capability::ExecTerminal,
        account_label: Some("alice".into()),
        command: None,
        requested_at: NOW,
        surface: Some("window".into()),
        older_client_terminal: false,
        request: None,
    };
    let ids = approval_candidates_from(&[request]);
    assert_eq!(values(&ids), ["ab".repeat(16)]);
    assert_eq!(helps(&ids), ["Laptop · terminal"]);

    // The command line completes the new command and its options.
    assert!(complete(&["se", "share", "grants", "acc"]).contains(&"access".to_string()));
    for (typed, option) in [
        ("--a", "--as"),
        ("--a", "--ask"),
        ("--c", "--commands"),
        ("--te", "--terminal"),
        ("--f", "--for"),
        ("--u", "--until"),
        ("--r", "--room"),
    ] {
        let options = complete(&["se", "share", "grants", "access", typed]);
        assert!(
            options.contains(&option.to_string()),
            "{option}: {options:?}"
        );
    }
}

/// With a running worker `--as` offers exactly the worker's account list
/// (U24 handover from U07), with the reason of an account that cannot be
/// chosen.
#[test]
fn completions_access_as_reads_the_worker_accounts() {
    use crate::share::exec_helper_protocol::{AccountChoice, AccountChoiceProfile, AccountReason};
    let mut remote = AccountChoice::other(AccountChoiceProfile::Remote, "se-remote");
    remote.refuse(
        AccountReason::HelperNotInstalled,
        "the account service is not set up on this PC".into(),
        None,
    );
    let deploy = AccountChoice::other(AccountChoiceProfile::Account, "deploy");
    let choices = [AccountChoice::own("alice"), remote, deploy];
    let accounts = account_candidates_from_choices(&choices);
    assert_eq!(values(&accounts), ["own", "remote", "account:deploy"]);
    let helps = helps(&accounts);
    assert_eq!(helps[0], "my account (alice)");
    assert!(helps[1].ends_with("not available: the account service is not set up on this PC"));
    assert_eq!(helps[2], "the local account deploy");
}
