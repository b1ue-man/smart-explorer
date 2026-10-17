//! Acceptance of the block of open requests and of „Zugriff für alle
//! Mitglieder…“ (PRM-M9) in real egui frames without a display, with the
//! harness of `share_access_ui_tests.rs`.
use std::collections::BTreeMap;

use eframe::egui;

use super::access_dialog::AccessDraft;
use super::access_ui_tests::{allowed, option, provider, Harness};
use super::{access_room, approvals_ui, exec_device_views};
use crate::share::access_approval::{ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalView};
use crate::share::access_catalog::HelperState;
use crate::share::access_grant::{AccountProfile, Capability, CapabilityDecision};
use crate::share::{
    ExecGrant, RoomMember, RoomMemberAdmission, RoomMemberFlags, RoomPolicy, RoomProfile,
    ShareProfiles, ShareStatus,
};

const NOW: i64 = 1_760_000_000;

fn request(id: u8, device: &str, capability: Capability, command: ApprovalCommand) -> ApprovalView {
    ApprovalView {
        id: ApprovalId::parse(format!("{id:02x}").repeat(16)).unwrap(),
        device_name: format!("Name {device}"),
        device_id: device.into(),
        fingerprint: format!("fp-{device}"),
        relation: "direct".into(),
        room: None,
        capability,
        account_label: Some("alice".into()),
        command: Some(command),
        requested_at: NOW,
        surface: Some("window".into()),
        older_client_terminal: false,
    }
}

#[test]
fn perm_task_gui_approvals_block() {
    let mut h = Harness::new();
    let shell = ApprovalCommand::Shell { text: "ls".into() };
    let requests = vec![
        request(0xaa, "a", Capability::ExecCommands, shell),
        request(0xbb, "b", Capability::ExecTerminal, ApprovalCommand::Login),
    ];
    let (first, second) = (requests[0].id.to_string(), requests[1].id.to_string());
    let mut armed = false;
    let mut draw = |ctx: &egui::Context| {
        let mut answer = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            answer = approvals_ui::block(ui, &requests, &|_| armed);
        });
        answer
    };
    h.settle(&mut draw);
    for label in [
        "OFFENE RÜCKFRAGEN (2)",
        "Name a möchte Befehle ausführen",
        "Name b möchte ein Terminal öffnen",
    ] {
        assert!(h.capture.contains(label), "{label}");
    }
    assert!(h
        .capture
        .contains("direkt verbunden · Befehle · als alice · Shell: ls"));
    // Before the delay only „Ablehnen“ answers.
    assert_eq!(h.click("Diesmal erlauben", 0, &mut draw), None);
    assert_eq!(
        h.click("Ablehnen", 0, &mut draw),
        Some((first.clone(), ApprovalDecision::Deny))
    );
    armed = true;
    let mut draw = |ctx: &egui::Context| {
        let mut answer = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            answer = approvals_ui::block(ui, &requests, &|_| armed);
        });
        answer
    };
    assert_eq!(
        h.click("Immer erlauben", 1, &mut draw),
        Some((second, ApprovalDecision::Always))
    );
    assert_eq!(
        h.click("Diesmal erlauben", 0, &mut draw),
        Some((first, ApprovalDecision::Once))
    );
    // No request: no block, no placeholder.
    let mut empty = |ctx: &egui::Context| {
        let mut answer = None;
        egui::CentralPanel::default()
            .show(ctx, |ui| answer = approvals_ui::block(ui, &[], &|_| true));
        answer
    };
    h.settle(&mut empty);
    assert!(!h
        .capture
        .labels
        .iter()
        .any(|(text, _, _)| text.starts_with("OFFENE RÜCKFRAGEN")));
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

#[test]
fn perm_task_gui_room_all_members() {
    let none = ExecGrant::default;
    let room = RoomProfile {
        id: "room-profile".into(),
        name: "Ops".into(),
        room_id: "room-wire".into(),
        auto_join: true,
        last_seen: None,
        status: ShareStatus::Waiting,
        members: vec![
            member("a", RoomMemberAdmission::Admitted, false, none()),
            member("b", RoomMemberAdmission::Admitted, false, allowed()),
            member("c", RoomMemberAdmission::Pending, true, none()),
            member("d", RoomMemberAdmission::Admitted, true, none()),
        ],
        exports: Default::default(),
        policy: RoomPolicy::new_room(),
    };
    let profiles = ShareProfiles {
        rooms: vec![room.clone()],
        ..ShareProfiles::default()
    };
    let views = exec_device_views(&profiles);
    // Only the members admitted now, each with its own revision.
    let members = access_room::snapshot(&views, &room);
    let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    let revisions: BTreeMap<&str, u64> = views
        .iter()
        .map(|v| (v.device_name.as_str(), v.policy_revision))
        .collect();
    assert_eq!(members[1].revision, revisions["b"]);

    let mut h = Harness::new();
    let user = provider(false, true);
    let accounts = (
        vec![option(AccountProfile::Own, true, None)],
        HelperState::NotNeeded,
    );
    let draft = AccessDraft::of(
        &room.id,
        &room.name,
        0,
        (Default::default(), Vec::new()),
        accounts,
    );
    access_room::open(
        &h.ctx,
        access_room::RoomDialog {
            draft,
            members: members.clone(),
        },
    );
    let mut draw = |ctx: &egui::Context| access_room::show(ctx, &user, &room.id, NOW);
    h.settle(&mut draw);
    assert!(h
        .capture
        .contains("Gilt für 2 zugelassene Mitglieder: a, b."));
    let scope =
        "Jedes Mitglied erhält ein eigenes Recht. Später zugelassene Mitglieder erhalten nichts.";
    assert!(h.capture.contains(scope));
    // One confirmation: the same change for both, each at its revision.
    let planned = h
        .click("Für 2 Mitglieder erlauben", 0, &mut draw)
        .expect("nothing planned");
    let keys: Vec<&str> = planned.iter().map(|(key, _, _)| key.as_str()).collect();
    assert_eq!(keys, [members[0].key.as_str(), members[1].key.as_str()]);
    assert_eq!(planned[0].1, planned[1].1);
    assert_eq!(
        (planned[0].2, planned[1].2),
        (members[0].revision, members[1].revision)
    );
    assert_eq!(planned[0].1.account, Some(AccountProfile::Own));
    assert_eq!(
        planned[0].1.capabilities[&Capability::ExecTerminal],
        CapabilityDecision::Allow
    );
    assert!(h.settle(&mut draw).is_none(), "the dialog stays open");
}
