//! W2 and G3 of the third review (2026-10-31), on the stored profiles:
//! „Abbrechen“ of a connection whose commands, terminal and screen wait for
//! the Exec-grant journal leaves it without any right (folders and write
//! right that were stored with the acceptance go too, a right the grant holds
//! now is recorded for the journal), and a waiting change that takes rights
//! away is in force at the worker without the journal step: the worker gets
//! the grant masked, the stored profile stays as it is.
use super::abgleich_task_tests::{accept, all_off, exec_on, host, journal_step, target_of, NOW};
use super::*;
use crate::share::access_grant::CapabilityDecision;
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, RightsOrigin, RightsPreset, RightsTarget, MENU_CAPABILITIES,
};

fn direct(target: &RightsTarget) -> (String, String) {
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = target
    else {
        panic!("a Direct target");
    };
    (device_id.clone(), public_key.clone())
}

fn rights_now(profiles: &ShareProfiles, target: &RightsTarget) -> ConnectionRights {
    let (device_id, public_key) = direct(target);
    profiles
        .current_direct_rights(&device_id, &public_key)
        .expect("the grant exists")
}

/// A connection that holds every right through the journal.
fn connected_with_everything() -> (ShareProfiles, RightsTarget) {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    journal_step(&mut profiles, NOW + 1);
    assert!(exec_on(&profiles, "peer").iter().all(|d| d.is_on()));
    assert!(profiles.pending_rights.is_empty());
    (profiles, target)
}

#[test]
fn abgleich_task_cancel_of_a_waiting_follow_up_leaves_no_folder_no_write_and_no_exec() {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    // The acceptance stored folders and write right; only exec waits.
    let before = rights_now(&profiles, &target);
    assert_eq!(before.folders, FolderChoice::All);
    assert!(before.write);
    assert!(profiles.exec_follow_ups_waiting());
    assert!(
        profiles.cancel_needs_journal(&target),
        "the follow-up waits"
    );

    assert!(profiles.cancel_connection_rights(&target, NOW + 1));
    let after = rights_now(&profiles, &target);
    assert!(!after.grants_anything(), "{after:?}");
    assert_eq!(after.folders, FolderChoice::None);
    assert!(!profiles.grant_for("peer").unwrap().write);
    assert!(all_off(&exec_on(&profiles, "peer")));
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
    // The connection itself stays.
    assert_eq!(
        profiles.grant_for("peer").unwrap().state,
        DirectGrantState::Accepted
    );
    assert!(!profiles.cancel_connection_rights(&target, NOW + 2));
    assert!(
        !profiles.cancel_needs_journal(&target),
        "nothing is open any more"
    );
}

#[test]
fn abgleich_task_cancel_of_a_narrowing_acceptance_records_the_revocation_of_what_is_held() {
    let (mut profiles, target) = connected_with_everything();
    let (device_id, public_key) = direct(&target);
    // A new request is accepted with „Nur lesen“: exec waits to be narrowed.
    profiles
        .apply_direct_connection_rights_deferring_exec(
            &device_id,
            &public_key,
            &RightsPreset::ReadOnly.rights(),
            RightsOrigin::Request,
            NOW + 2,
        )
        .unwrap();
    assert!(profiles.pending_rights_of(&target).is_some());
    assert!(exec_on(&profiles, "peer").iter().all(|d| d.is_on()));

    // „Abbrechen“ does not wait for the journal to take folders and write.
    assert!(profiles.cancel_needs_journal(&target), "exec is held");
    assert!(profiles.cancel_connection_rights(&target, NOW + 3));
    assert_eq!(rights_now(&profiles, &target).folders, FolderChoice::None);
    assert!(!profiles.grant_for("peer").unwrap().write);
    // Commands, terminal and screen go only through the journal: the
    // revocation is recorded and the connection stays „Rechte offen“.
    assert!(exec_on(&profiles, "peer").iter().all(|d| d.is_on()));
    assert_eq!(profiles.exec_follow_ups.len(), 1);
    assert!(profiles.pending_rights_of(&target).is_some());
    let masked = profiles
        .masked_by_narrowing_follow_ups(NOW + 4)
        .expect("the revocation narrows the grant");
    assert!(!masked.grant_for("peer").unwrap().exec.grants_any());
    // The journal step stores it and ends the open connection.
    journal_step(&mut profiles, NOW + 5);
    assert!(all_off(&exec_on(&profiles, "peer")));
    assert!(profiles.exec_follow_ups.is_empty());
    assert!(profiles.pending_rights.is_empty());
}

#[test]
fn abgleich_task_cancel_rights_keep_account_and_end_and_turn_every_decision_off() {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(ConnectionRights {
            valid_until: Some(NOW + 1000),
            ..RightsPreset::All.rights()
        }))
        .unwrap();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    journal_step(&mut profiles, NOW + 1);
    let held = rights_now(&profiles, &target);
    let cancelled = profiles.rights_after_cancel(&target);
    assert_eq!(cancelled.folders, FolderChoice::None);
    assert!(!cancelled.write);
    assert_eq!(cancelled.valid_until, held.valid_until);
    assert_eq!(cancelled.account, held.account);
    for capability in MENU_CAPABILITIES {
        assert_eq!(cancelled.decision(capability), CapabilityDecision::Off);
    }
}

#[test]
fn abgleich_task_narrowing_change_is_masked_for_the_worker_but_a_widening_is_not() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_narrowing_change_is_masked_for_the_worker_but_a_widening_is_not",
            abgleich_task_narrowing_change_is_masked_for_the_worker_but_a_widening_is_not_case,
        ),
        (
            "abgleich_task_a_change_that_narrows_one_right_and_widens_another_is_both",
            abgleich_task_a_change_that_narrows_one_right_and_widens_another_is_both,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn abgleich_task_narrowing_change_is_masked_for_the_worker_but_a_widening_is_not_case() {
    // Widening: a new connection with the Direct code's rights.
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::All.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let effect = profiles.direct_follow_up_effect("peer", NOW);
    assert!(effect.widens && !effect.narrows, "{effect:?}");
    assert!(
        profiles.masked_by_narrowing_follow_ups(NOW).is_none(),
        "nothing is held that could be narrowed"
    );

    // Narrowing: the journal step has not run (or failed); the stored grant
    // still holds everything, the worker gets none of it.
    let (mut profiles, target) = connected_with_everything();
    let (device_id, public_key) = direct(&target);
    profiles
        .apply_direct_connection_rights_deferring_exec(
            &device_id,
            &public_key,
            &RightsPreset::ReadOnly.rights(),
            RightsOrigin::Request,
            NOW + 2,
        )
        .unwrap();
    let effect = profiles.direct_follow_up_effect("peer", NOW + 2);
    assert!(effect.narrows && !effect.widens, "{effect:?}");
    let stored = profiles.clone();
    let masked = profiles
        .masked_by_narrowing_follow_ups(NOW + 2)
        .expect("a narrowing change waits");
    assert!(!masked.grant_for("peer").unwrap().exec.grants_any());
    assert!(all_off(&exec_on(&masked, "peer")));
    assert_eq!(
        masked.grant_for("peer").unwrap().exec.policy_revision,
        stored.grant_for("peer").unwrap().exec.policy_revision,
        "a mask is no new revision"
    );
    assert_eq!(
        profiles.grant_for("peer").unwrap().exec,
        stored.grant_for("peer").unwrap().exec,
        "the stored grant is not touched"
    );
    assert!(profiles.grant_for("peer").unwrap().exec.grants_any());
}

fn abgleich_task_a_change_that_narrows_one_right_and_widens_another_is_both() {
    let mut profiles = host();
    profiles
        .set_code_rights(Some(RightsPreset::FilesExec.rights()))
        .unwrap();
    accept(&mut profiles, NOW);
    let target = target_of(&profiles, "peer");
    journal_step(&mut profiles, NOW + 1);
    let (device_id, public_key) = direct(&target);
    // Terminal off, desktop view on: one right goes, another comes.
    let mixed = ConnectionRights {
        terminal: CapabilityDecision::Off,
        screen_view: CapabilityDecision::Allow,
        ..RightsPreset::FilesExec.rights()
    };
    profiles
        .apply_direct_connection_rights_deferring_exec(
            &device_id,
            &public_key,
            &mixed,
            RightsOrigin::Request,
            NOW + 2,
        )
        .unwrap();
    let effect = profiles.direct_follow_up_effect("peer", NOW + 2);
    assert!(effect.narrows && effect.widens, "{effect:?}");
    assert!(profiles.masked_by_narrowing_follow_ups(NOW + 2).is_some());
}
