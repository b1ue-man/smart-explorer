//! Host tests of the exec-host facade: target keys and their resolution,
//! the JSON of `share.status` / `share.execJobs` and the argument and
//! result rules of `share.setExec` / `share.cancelExecJob` (no worker).
use serde_json::{json, Value};

use super::share_exec::{cancel_target, grant_answer, jobs_json, targets_json};
use crate::daemon::{ExecJobDirection, ExecJobsSnapshot};
use crate::share::{
    resolve_exec_target, DirectGrant, DirectGrantState, ExecGrant, ExecGrantTarget, ExecId,
    ExecJobView, ExecLifecycleState, ExecTerminal, ExecTerminalKind, RoomMember, RoomProfile,
    ShareProfiles, ShareStatus,
};

fn grant(device_id: &str, name: &str, state: DirectGrantState, enabled: bool) -> DirectGrant {
    DirectGrant {
        device_id: device_id.into(),
        device_name: name.into(),
        public_key: format!("key-{device_id}"),
        fingerprint: format!("fp-{device_id}"),
        node_id: format!("node-{device_id}"),
        state,
        updated_at: 10,
        exec: ExecGrant {
            enabled,
            policy_revision: if enabled { 7 } else { 0 },
            ..ExecGrant::default()
        },
    }
}

fn member(device_id: &str, name: &str, blocked: bool) -> RoomMember {
    RoomMember {
        device_id: device_id.into(),
        device_name: name.into(),
        fingerprint: format!("fp-{device_id}"),
        public_key: format!("key-{device_id}"),
        node_id: format!("node-{device_id}"),
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status: ShareStatus::Waiting,
        blocked,
        exec: ExecGrant::default(),
        presence: None,
    }
}

fn profiles() -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(grant(
        "d-laptop",
        "Laptop",
        DirectGrantState::Accepted,
        true,
    ));
    profiles
        .direct_grants
        .push(grant("d-alt", "Altgerät", DirectGrantState::Ignored, false));
    profiles.rooms.push(RoomProfile {
        id: "profile-team".into(),
        name: "Team".into(),
        room_id: "wire-team".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![
            member("d-desktop", "Desktop", false),
            member("0123456789abcdef", " ", true),
        ],
        exports: Default::default(),
    });
    profiles
}

fn by_key<'a>(targets: &'a [Value], key: &str) -> &'a Value {
    targets
        .iter()
        .find(|target| target["targetKey"] == key)
        .unwrap_or_else(|| panic!("no target {key}: {targets:?}"))
}

#[test]
fn android_task_exec_targets_list_every_grant_and_member_with_state() {
    let targets = targets_json(&profiles());
    let names: Vec<&str> = targets
        .iter()
        .map(|target| target["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["Altgerät", "Desktop", "Geraet 01234567", "Laptop"]);

    assert_eq!(
        by_key(&targets, "direct/d-laptop/fp-d-laptop"),
        &json!({
            "targetKey": "direct/d-laptop/fp-d-laptop", "relation": "direct", "roomId": null,
            "roomName": null, "deviceId": "d-laptop", "name": "Laptop",
            "fingerprint": "fp-d-laptop", "enabled": true, "baseAuthorized": true,
            "policyRevision": 7,
        })
    );
    let ignored = by_key(&targets, "direct/d-alt/fp-d-alt");
    assert_eq!(ignored["baseAuthorized"], false);
    let desktop = by_key(&targets, "room/wire-team/d-desktop/fp-d-desktop");
    assert_eq!(desktop["relation"], "room");
    assert_eq!(desktop["roomId"], "wire-team");
    assert_eq!(desktop["roomName"], "Team");
    assert_eq!(desktop["enabled"], false);
    assert_eq!(desktop["baseAuthorized"], true);
    let blocked = by_key(
        &targets,
        "room/wire-team/0123456789abcdef/fp-0123456789abcdef",
    );
    assert_eq!(blocked["baseAuthorized"], false);
}

#[test]
fn android_task_exec_target_keys_resolve_only_the_current_identity() {
    let mut profiles = profiles();
    let key = "room/wire-team/d-desktop/fp-d-desktop";
    let view = resolve_exec_target(&profiles, key).expect("current member");
    assert_eq!(
        view.target,
        ExecGrantTarget::RoomMember {
            room_id: "wire-team".into(),
            device_id: "d-desktop".into(),
            public_key: "key-d-desktop".into(),
            fingerprint: "fp-d-desktop".into(),
            node_id: "node-d-desktop".into(),
        }
    );
    assert!(view.base_authorized);
    assert!(resolve_exec_target(&profiles, "direct/d-desktop/fp-d-desktop").is_none());
    assert!(resolve_exec_target(&profiles, "room/wire-team/d-desktop").is_none());

    // The device came back with a new identity: the old key names nobody.
    profiles.rooms[0].members[0].fingerprint = "fp-new".into();
    assert!(resolve_exec_target(&profiles, key).is_none());
    let renewed =
        resolve_exec_target(&profiles, "room/wire-team/d-desktop/fp-new").expect("renewed member");
    assert_eq!(renewed.fingerprint, "fp-new");

    // Two identities behind one key are ambiguous: nothing is picked.
    let mut twin = profiles.direct_grants[0].clone();
    twin.public_key = "key-other".into();
    profiles.direct_grants.push(twin);
    assert!(resolve_exec_target(&profiles, "direct/d-laptop/fp-d-laptop").is_none());
}

#[test]
fn android_task_exec_grant_counts_only_when_stored_and_applied() {
    assert_eq!(
        grant_answer(true, true, None, 12).expect("complete"),
        json!({ "revision": 12 })
    );
    let pending = grant_answer(true, false, None, 12).expect_err("not applied");
    assert_eq!(pending.kind, "internal");
    assert!(
        pending
            .message
            .contains("gespeichert: ja, angewendet: nein"),
        "{}",
        pending.message
    );
    let failed = grant_answer(false, false, Some("Datenträger voll"), 3).expect_err("failed");
    assert!(
        failed.message.ends_with("Datenträger voll"),
        "{}",
        failed.message
    );
    assert!(grant_answer(true, true, Some("Journal"), 4).is_err());
}

fn job(
    id: &str,
    state: ExecLifecycleState,
    terminal: Option<(ExecTerminalKind, i32)>,
) -> ExecJobView {
    let exec_id = ExecId::parse(id.repeat(16)).expect("exec id");
    ExecJobView {
        exec_id: exec_id.clone(),
        peer_device_id: "d-desktop".into(),
        peer_device_name: "Desktop".into(),
        program: "<shell>".into(),
        command_digest: "digest".into(),
        state,
        policy_revision: 3,
        started_at: Some(1_700_000_000),
        finished_at: terminal.as_ref().map(|_| 1_700_000_005),
        terminal: terminal.map(|(kind, code)| ExecTerminal {
            exec_id,
            kind,
            exit_code: Some(code),
            signal: None,
            message: Some("beendet".into()),
            stdout_bytes: 0,
            stderr_bytes: 0,
            output_truncated: false,
        }),
    }
}

#[test]
fn android_task_exec_jobs_json_lists_incoming_first_with_snake_case_states() {
    let snapshot = ExecJobsSnapshot {
        incoming_active: vec![job("aa", ExecLifecycleState::Running, None)],
        outgoing_active: vec![job("bb", ExecLifecycleState::QueuedLocal, None)],
        incoming_history: vec![
            job(
                "cc",
                ExecLifecycleState::TimedOut,
                Some((ExecTerminalKind::TimedOut, 137)),
            ),
            job(
                "dd",
                ExecLifecycleState::Exited,
                Some((ExecTerminalKind::Exited, 0)),
            ),
        ],
        outgoing_history: vec![],
    };
    let jobs = jobs_json(&snapshot);
    assert_eq!(
        jobs["active"][0],
        json!({
            "direction": "incoming", "execId": "aa".repeat(16), "peerDeviceId": "d-desktop",
            "peerName": "Desktop", "program": "<shell>", "state": "running",
            "startedAt": 1_700_000_000, "finishedAt": null, "exitCode": null, "message": null,
        })
    );
    assert_eq!(jobs["active"][1]["direction"], "outgoing");
    assert_eq!(jobs["active"][1]["state"], "queued_local");
    let history = jobs["history"].as_array().expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["state"], "timed_out");
    assert_eq!(history[0]["exitCode"], 137);
    assert_eq!(history[0]["finishedAt"], 1_700_000_005);
    assert_eq!(history[1]["state"], "exited");
    assert_eq!(history[1]["message"], "beendet");
    assert_eq!(
        jobs_json(&ExecJobsSnapshot::default()),
        json!({ "active": [], "history": [] })
    );
}

#[test]
fn android_task_exec_cancel_names_direction_job_and_peer() {
    let target = cancel_target(&json!({
        "direction": "incoming", "execId": "AB".repeat(16), "peerDeviceId": "d-desktop",
    }))
    .expect("target");
    assert_eq!(target.direction, ExecJobDirection::Incoming);
    assert_eq!(target.exec_id.as_str(), "ab".repeat(16));
    assert_eq!(target.peer_device_id, "d-desktop");
    let outgoing = cancel_target(&json!({
        "direction": "outgoing", "execId": "01".repeat(16), "peerDeviceId": "p",
    }))
    .expect("outgoing");
    assert_eq!(outgoing.direction, ExecJobDirection::Outgoing);
    for args in [
        json!({ "direction": "sideways", "execId": "01".repeat(16), "peerDeviceId": "p" }),
        json!({ "direction": "incoming", "execId": "kurz", "peerDeviceId": "p" }),
        json!({ "direction": "incoming", "execId": "01".repeat(16) }),
    ] {
        assert_eq!(cancel_target(&args).expect_err("invalid").kind, "invalid");
    }
}
