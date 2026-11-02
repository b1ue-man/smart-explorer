//! W8 (review 2026-10-31): „Bestätigen“ sets commands, terminal and screen
//! through the Exec-grant journal: the entry names exactly the changed
//! grants and the rest of the confirmation, a revocation is a pending deny,
//! persisting it is all or nothing and can be repeated, a concurrent grant
//! change is refused, and daemons that know only version 2 refuse it.
use super::super::{decode_entry, JournalPhase, CONFIRM_JOURNAL_VERSION, JOURNAL_VERSION};
use super::{apply_persisted, confirm_entry, grant_steps, RightsStep};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::connection_rights::{ConnectionRights, FolderChoice, RightsPreset, RightsTarget};
use crate::share::connection_rights_actions::plan_confirm;
use crate::share::{
    DirectGrant, DirectGrantState, DirectPeerIdentity, ExecGrant, ShareIdentity, ShareProfiles,
    SharedRoot, DIRECT_EXPORT_SCOPE,
};

fn identity() -> ShareIdentity {
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

/// A host with two Direct folders and one pinned, accepted device that got
/// „Alles“ at time 10.
fn host_with_all() -> (ShareProfiles, RightsTarget) {
    let peer =
        DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]));
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![
        SharedRoot::new_in_scope("Docs", "/home/u/Docs", DIRECT_EXPORT_SCOPE),
        SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", DIRECT_EXPORT_SCOPE),
    ];
    profiles.direct_grants.push(DirectGrant {
        device_id: peer.device_id.clone(),
        device_name: peer.device_name.clone(),
        public_key: peer.public_key.clone(),
        fingerprint: peer.fingerprint.clone(),
        node_id: peer.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant::default(),
        write: false,
    });
    let target = RightsTarget::Direct {
        device_id: peer.device_id,
        public_key: peer.public_key,
    };
    profiles
        .confirm_connection_rights(&target, &RightsPreset::All.rights(), 10)
        .expect("all");
    (profiles, target)
}

fn step(target: &RightsTarget, rights: ConnectionRights, at: i64) -> RightsStep {
    RightsStep {
        target: target.clone(),
        rights,
        added: Vec::new(),
        changed_at: at,
    }
}

fn exec_of(profiles: &ShareProfiles) -> &ExecGrant {
    &profiles.direct_grants[0].exec
}

#[test]
fn abgleich_task_confirm_revocation_is_one_pending_deny_entry() {
    let (profiles, target) = host_with_all();
    let identity = identity();
    let none = RightsPreset::None.rights();
    let mut desired = profiles.clone();
    plan_confirm(&mut desired, &target, &none, &[], 20).unwrap();
    let steps = grant_steps(&profiles, &desired, &identity).unwrap();
    assert_eq!(steps.len(), 1, "exactly the device's grant");
    let entry = confirm_entry(steps, step(&target, none.clone(), 20))
        .unwrap()
        .expect("an Exec change needs the journal");
    assert_eq!(entry.version, CONFIRM_JOURNAL_VERSION);
    assert_eq!(entry.phase, JournalPhase::PendingDeny, "revocation first");
    assert!(!entry.mutation.policy.grants_any());
    assert_eq!(
        entry.expected_policy_revision,
        exec_of(&profiles).policy_revision
    );
    // Durable form: what is written decodes to the same entry.
    let encoded = serde_json::to_vec(&entry).unwrap();
    let decoded = decode_entry(&encoded).unwrap();
    assert_eq!(decoded, entry);
    decoded.validate().unwrap();

    // Granting again is a pending apply (persisted before the worker).
    let mut granted = desired.clone();
    plan_confirm(
        &mut granted,
        &target,
        &RightsPreset::FilesExec.rights(),
        &[],
        30,
    )
    .unwrap();
    let steps = grant_steps(&desired, &granted, &identity).unwrap();
    let entry = confirm_entry(steps, step(&target, RightsPreset::FilesExec.rights(), 30))
        .unwrap()
        .unwrap();
    assert_eq!(entry.phase, JournalPhase::PendingApply);
}

#[test]
fn abgleich_task_confirm_without_exec_change_needs_no_journal() {
    let (profiles, target) = host_with_all();
    let only_docs = ConnectionRights {
        folders: FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false,
        },
        ..RightsPreset::All.rights()
    };
    let mut desired = profiles.clone();
    plan_confirm(&mut desired, &target, &only_docs, &[], 20).unwrap();
    let steps = grant_steps(&profiles, &desired, &identity()).unwrap();
    assert!(
        steps.is_empty(),
        "folders alone are one profile transaction"
    );
    assert!(confirm_entry(steps, step(&target, only_docs, 20))
        .unwrap()
        .is_none());
}

#[test]
fn abgleich_task_confirm_persists_all_or_nothing_and_repeats_safely() {
    let (profiles, target) = host_with_all();
    let identity = identity();
    let none = RightsPreset::None.rights();
    let mut desired = profiles.clone();
    plan_confirm(&mut desired, &target, &none, &[], 20).unwrap();
    let entry = confirm_entry(
        grant_steps(&profiles, &desired, &identity).unwrap(),
        step(&target, none.clone(), 20),
    )
    .unwrap()
    .unwrap();

    let mut stored = profiles.clone();
    apply_persisted(&entry, &mut stored, &identity).unwrap();
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = &target
    else {
        unreachable!()
    };
    assert_eq!(
        stored.current_direct_rights(device_id, public_key),
        Some(none.clone()),
        "grant, folders and write in one step"
    );
    assert_eq!(exec_of(&stored), exec_of(&desired));
    for capability in [
        Capability::ExecCommands,
        Capability::ExecTerminal,
        Capability::ScreenView,
        Capability::ScreenControl,
    ] {
        assert_eq!(
            exec_of(&stored).decision_at(capability, 20),
            CapabilityDecision::Off
        );
    }
    // A recovery repeats the entry: nothing changes any more.
    let again = stored.clone();
    apply_persisted(&entry, &mut stored, &identity).unwrap();
    assert_eq!(stored.direct_grants, again.direct_grants);
    assert_eq!(stored.default_direct_exports, again.default_direct_exports);

    // A grant change in between (another revision) refuses the entry, so
    // it never overwrites a newer grant.
    let mut changed = profiles.clone();
    changed
        .confirm_connection_rights(&target, &RightsPreset::FilesExec.rights(), 15)
        .unwrap();
    let before = changed.clone();
    assert!(apply_persisted(&entry, &mut changed, &identity).is_err());
    assert_eq!(changed.direct_grants, before.direct_grants);
}

#[test]
fn abgleich_task_confirm_entries_are_refused_by_version_two_readers() {
    let (profiles, target) = host_with_all();
    let identity = identity();
    let none = RightsPreset::None.rights();
    let mut desired = profiles.clone();
    plan_confirm(&mut desired, &target, &none, &[], 20).unwrap();
    let entry = confirm_entry(
        grant_steps(&profiles, &desired, &identity).unwrap(),
        step(&target, none, 20),
    )
    .unwrap()
    .unwrap();
    let encoded: serde_json::Value = serde_json::to_value(&entry).unwrap();
    // Daemons up to 0.5.189 accept only version 2 and refuse everything
    // else before they act on it (fail closed).
    assert_eq!(encoded["version"], CONFIRM_JOURNAL_VERSION);
    assert_ne!(CONFIRM_JOURNAL_VERSION, JOURNAL_VERSION);
    assert!(encoded.get("rights").is_some());
    // A confirmation is never valid as version 2, nor a single grant
    // entry with further grants.
    let mut downgraded = entry.clone();
    downgraded.version = JOURNAL_VERSION;
    assert!(downgraded.validate().is_err());
    let mut widened = entry.clone();
    widened.rights = None;
    widened.version = JOURNAL_VERSION;
    widened.more = vec![super::GrantStep {
        mutation: entry.mutation.clone(),
        expected_policy_revision: entry.expected_policy_revision,
    }];
    assert!(widened.validate().is_err());
    // A plain grant entry keeps version 2 and its form.
    let plain =
        super::super::JournalEntry::new(entry.mutation.clone(), entry.expected_policy_revision)
            .unwrap();
    let plain_json = serde_json::to_value(&plain).unwrap();
    assert_eq!(plain_json["version"], JOURNAL_VERSION);
    assert!(plain_json.get("rights").is_none() && plain_json.get("more").is_none());
}
