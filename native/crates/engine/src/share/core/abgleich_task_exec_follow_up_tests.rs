//! Review 2, follow-up of W8 (2026-10-31): commands, terminal and screen
//! reach a connection only through the Exec-grant journal. Every profile
//! transaction outside the journal (acceptance with rights, the rights of
//! the Direct code on first connect and on re-admission, a PIN offer's
//! rights, the rights chosen for a room, a room's member access, the
//! automatic acceptance) stores everything else and records the change; the
//! journal's persistence step stores it and ends the record, a cancelled or
//! superseded record is never applied, and the source holds no other writer.
use super::*;
use crate::share::access_grant::CapabilityDecision;
use crate::share::connection_rights::{
    FolderChoice, RightsOrigin, RightsPreset, RightsTarget, MENU_CAPABILITIES,
};
use crate::share::direct_protocol::DirectDecisionKind;
use crate::share::{
    DirectPeerIdentity, ExecGrantMutation, ShareIdentity, SharedRoot, DIRECT_EXPORT_SCOPE,
};

// The same fixture as `direct_ledger_tests.rs`, compiled here once more.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "direct_ledger_fixture.rs"]
mod ledger;

pub(super) const NOW: i64 = 100;

pub(super) fn identity() -> ShareIdentity {
    let iroh_secret = iroh::SecretKey::from_bytes(&[1; 32]);
    let local = DirectPeerIdentity::from_secret("local-device", "Local", &iroh_secret);
    ShareIdentity {
        device_id: local.device_id,
        device_name: local.device_name,
        direct_lookup_id: "local-lookup".into(),
        public_key: local.public_key,
        fingerprint: local.fingerprint,
        node_id: local.node_id,
        iroh_secret,
        direct_secret: [7; 32],
    }
}

pub(super) fn peer() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]))
}

pub(super) fn host() -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![
        SharedRoot::new_in_scope("Docs", "/home/u/Docs", DIRECT_EXPORT_SCOPE),
        SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", DIRECT_EXPORT_SCOPE),
    ];
    profiles
}

pub(super) fn accept(profiles: &mut ShareProfiles, at: i64) {
    crate::share::legacy_direct_request_decision::set_exact_grant(profiles, &peer(), true, at)
        .unwrap();
}

pub(super) fn revoke(profiles: &mut ShareProfiles, at: i64) {
    crate::share::legacy_direct_request_decision::set_exact_grant(profiles, &peer(), false, at)
        .unwrap();
}

pub(super) fn target_of(profiles: &ShareProfiles, device: &str) -> RightsTarget {
    let grant = profiles.grant_for(device).expect("grant");
    RightsTarget::Direct {
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
    }
}

pub(super) fn exec_on(profiles: &ShareProfiles, device: &str) -> Vec<CapabilityDecision> {
    let grant = profiles.grant_for(device).expect("grant");
    MENU_CAPABILITIES
        .into_iter()
        .map(|capability| grant.exec.decision_at(capability, NOW))
        .collect()
}

pub(super) fn all_off(decisions: &[CapabilityDecision]) -> bool {
    decisions.iter().all(|decision| !decision.is_on())
}

/// What the persistence step of the Exec-grant journal does with the next
/// waiting record: the grant is set (compare-and-set against its revision)
/// and the record ends (`exec_grant_journal_rights.rs`, `apply_persisted`).
pub(super) fn journal_step(profiles: &mut ShareProfiles, at: i64) -> ExecGrantMutation {
    let next = profiles.next_exec_follow_up().expect("a record waits");
    let FollowUpWork::Apply { target, change } = next.work else {
        panic!("the record must be applicable: {:?}", next.work);
    };
    let identity = identity();
    let mut scratch = profiles.clone();
    let (mutation, expected) =
        ExecGrantMutation::prepare_access(&mut scratch, &identity, target, &change, None, at)
            .unwrap();
    mutation
        .apply_persisted_cas(profiles, &identity, expected)
        .unwrap();
    profiles.finish_exec_follow_ups(&mutation.target);
    mutation
}

#[test]
fn abgleich_task_follow_up_code_rights_reach_the_grant_only_through_the_journal() {
    let cases: [(&str, fn()); 3] = [
        (
            "abgleich_task_follow_up_code_rights_reach_the_grant_only_through_the_journal",
            abgleich_task_follow_up_code_rights_reach_the_grant_only_through_the_journal_case,
        ),
        (
            "abgleich_task_follow_up_readmitted_device_regains_exec_only_through_the_journal",
            abgleich_task_follow_up_readmitted_device_regains_exec_only_through_the_journal,
        ),
        (
            "abgleich_task_follow_up_signed_acceptance_with_code_rights",
            abgleich_task_follow_up_signed_acceptance_with_code_rights,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn abgleich_task_follow_up_code_rights_reach_the_grant_only_through_the_journal_case() {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let grant = profiles.grant_for("peer").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert!(grant.write, "write is stored with the acceptance");
    assert!(!grant.exec.grants_any(), "commands and screen are not");
    assert!(all_off(&exec_on(&profiles, "peer")));
    let target = target_of(&profiles, "peer");
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    assert_eq!(
        profiles
            .current_direct_rights(device_id, public_key)
            .unwrap()
            .folders,
        FolderChoice::All,
        "folders are stored with the acceptance"
    );
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    assert!(
        profiles.pending_rights_of(&target).is_some(),
        "the connection stays open until the journal stored it"
    );

    let mutation = journal_step(&mut profiles, NOW + 1);
    assert!(mutation.policy.grants_any());
    assert!(exec_on(&profiles, "peer").iter().all(|d| d.is_on()));
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
}

fn abgleich_task_follow_up_readmitted_device_regains_exec_only_through_the_journal() {
    let mut profiles = host();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    profiles
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), NOW)
        .unwrap();
    assert!(exec_on(&profiles, "peer").iter().all(|d| d.is_on()));
    revoke(&mut profiles, NOW + 1);
    assert!(all_off(&exec_on(&profiles, "peer")));
    profiles
        .set_code_rights(Some(RightsPreset::FilesExec.rights()))
        .unwrap();
    accept(&mut profiles, NOW + 2);
    assert!(
        all_off(&exec_on(&profiles, "peer")),
        "the old rights do not return, the code's rights wait for the journal"
    );
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    assert_eq!(
        profiles.pending_rights_of(&target).map(|p| p.origin),
        Some(RightsOrigin::Readmitted)
    );
    journal_step(&mut profiles, NOW + 3);
    let decisions = exec_on(&profiles, "peer");
    assert!(
        decisions[0].is_on() && decisions[1].is_on(),
        "commands, terminal"
    );
    assert!(!decisions[2].is_on() && !decisions[3].is_on(), "no screen");
}

fn abgleich_task_follow_up_signed_acceptance_with_code_rights() {
    let request = ledger::request(None);
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::FilesExec.rights()))
        .unwrap();
    profiles
        .record_incoming_direct_request("lookup-a", request.clone(), 110)
        .unwrap();
    profiles
        .record_direct_decision(
            ledger::decision(&request, DirectDecisionKind::Accepted, 1, 130),
            130,
        )
        .unwrap();
    let grant = profiles.grant_for("requester-a").unwrap();
    assert_eq!(grant.state, DirectGrantState::Accepted);
    assert!(grant.write && !grant.exec.grants_any());
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    journal_step(&mut profiles, 140);
    assert!(profiles.grant_for("requester-a").unwrap().exec.grants_any());
}

#[test]
fn abgleich_task_follow_up_explicit_rights_change_exec_only_through_the_journal() {
    let mut profiles = host();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    // „Dateien und Codeausführung“ with the acceptance.
    profiles
        .apply_direct_connection_rights_deferring_exec(
            device_id,
            public_key,
            &RightsPreset::FilesExec.rights(),
            RightsOrigin::Request,
            NOW + 1,
        )
        .unwrap();
    assert!(profiles.grant_for("peer").unwrap().write);
    assert!(all_off(&exec_on(&profiles, "peer")));
    journal_step(&mut profiles, NOW + 2);
    assert!(exec_on(&profiles, "peer")[0].is_on());

    // The revocation is a record too: the grant keeps what it has until the
    // journal revokes it (phase „pending deny“: worker first).
    profiles
        .apply_direct_connection_rights_deferring_exec(
            device_id,
            public_key,
            &RightsPreset::None.rights(),
            RightsOrigin::Request,
            NOW + 3,
        )
        .unwrap();
    assert!(
        exec_on(&profiles, "peer")[0].is_on(),
        "not revoked in the profile"
    );
    let mutation = journal_step(&mut profiles, NOW + 4);
    assert!(!mutation.policy.grants_any());
    assert!(all_off(&exec_on(&profiles, "peer")));

    // Rights without commands, terminal or screen need no record at all.
    profiles
        .apply_direct_connection_rights_deferring_exec(
            device_id,
            public_key,
            &RightsPreset::ReadOnly.rights(),
            RightsOrigin::Request,
            NOW + 5,
        )
        .unwrap();
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
}

#[test]
fn abgleich_task_follow_up_cancel_confirm_and_explicit_change_end_the_record() {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    // „Abbrechen“: the connection stays without those rights, also without
    // the folders and the write right that were stored with the acceptance
    // (W2, review 3).
    assert!(profiles.cancel_connection_rights(&target, NOW + 1));
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
    assert!(all_off(&exec_on(&profiles, "peer")));
    assert!(!profiles.grant_for("peer").unwrap().write);
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    assert_eq!(
        profiles
            .current_direct_rights(device_id, public_key)
            .unwrap()
            .folders,
        FolderChoice::None
    );

    // „Bestätigen“ (the journal's step) supersedes the record.
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    revoke(&mut profiles, NOW + 1);
    accept(&mut profiles, NOW + 2);
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    profiles
        .confirm_connection_rights(&target, &RightsPreset::ReadWrite.rights(), NOW + 3)
        .unwrap();
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(all_off(&exec_on(&profiles, "peer")));

    // An explicit change of the grant (the journal) supersedes it as well.
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    revoke(&mut profiles, NOW + 4);
    accept(&mut profiles, NOW + 5);
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    let grant_target = {
        let grant = profiles.grant_for("peer").unwrap();
        crate::share::ExecGrantTarget::Direct {
            device_id: grant.device_id.clone(),
            public_key: grant.public_key.clone(),
            fingerprint: grant.fingerprint.clone(),
            node_id: grant.node_id.clone(),
        }
    };
    profiles.finish_exec_follow_ups(&grant_target);
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
}
