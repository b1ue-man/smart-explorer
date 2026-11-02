//! W2 and G3 of the third review (2026-10-31), at the Exec-grant journal:
//! „Abbrechen“ of a connection that holds or waits for commands, terminal or
//! screen is the confirmation of „no rights“ (a revocation: the worker first,
//! then stored, together with folders, write right and the end of „Rechte
//! offen“), and a step that only narrows a grant reaches the running worker
//! before the confirmation is stored, so it holds when the storing fails.
use super::*;
use crate::share::connection_rights::{ConnectionRights, FolderChoice, RightsOrigin, RightsPreset};
use crate::share::{
    DirectGrant, DirectGrantState, DirectPeerIdentity, ExecGrant, SharedRoot, DIRECT_EXPORT_SCOPE,
};

/// A source file as these checks read it: a Windows checkout has CRLF line
/// ends (no `.gitattributes` rule fixes `*.rs`), the patterns below are
/// written with `\n`.
fn source(text: &str) -> String {
    text.replace("\r\n", "\n")
}

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

/// An accepted, fully pinned Direct grant without any right, and one Direct
/// folder.
fn host() -> (ShareProfiles, RightsTarget) {
    let peer =
        DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]));
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![SharedRoot::new_in_scope(
        "Docs",
        "/home/u/Docs",
        DIRECT_EXPORT_SCOPE,
    )];
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
    (profiles, target)
}

fn confirm(profiles: &mut ShareProfiles, target: &RightsTarget, rights: &ConnectionRights) {
    actions::plan_confirm(profiles, target, rights, &[], 10).unwrap();
}

/// The journal entry of the confirmation of `rights` on top of `from`.
fn entry_for(
    from: &ShareProfiles,
    target: &RightsTarget,
    rights: &ConnectionRights,
) -> JournalEntry {
    let mut desired = from.clone();
    actions::plan_confirm(&mut desired, target, rights, &[], 20).unwrap();
    let steps = grant_steps(from, &desired, &identity()).unwrap();
    let step = RightsStep {
        target: target.clone(),
        rights: rights.clone(),
        added: Vec::new(),
        changed_at: 20,
    };
    confirm_entry(steps, step)
        .unwrap()
        .expect("a grant changes")
}

#[test]
fn abgleich_task_cancel_of_a_connection_holding_exec_is_a_revoking_journal_entry() {
    let (mut profiles, target) = host();
    confirm(&mut profiles, &target, &RightsPreset::All.rights());
    let RightsTarget::Direct {
        device_id,
        public_key,
    } = target.clone()
    else {
        unreachable!()
    };
    // A new request was accepted with „Nur lesen“: exec waits to be narrowed.
    profiles
        .apply_direct_connection_rights_deferring_exec(
            &device_id,
            &public_key,
            &RightsPreset::ReadOnly.rights(),
            RightsOrigin::Request,
            11,
        )
        .unwrap();
    assert!(profiles.pending_rights_of(&target).is_some());
    assert!(profiles.cancel_needs_journal(&target));

    let rights = profiles.rights_after_cancel(&target);
    assert!(!rights.grants_anything(), "{rights:?}");
    let entry = entry_for(&profiles, &target, &rights);
    assert_eq!(entry.phase, JournalPhase::PendingDeny, "the worker first");
    entry.validate().unwrap();

    let mut stored = profiles.clone();
    apply_persisted(&entry, &mut stored, &identity()).unwrap();
    let grant = stored.grant_for("peer").unwrap();
    assert!(!grant.exec.grants_any());
    assert!(!grant.write);
    assert_eq!(
        stored
            .current_direct_rights(&device_id, &public_key)
            .unwrap()
            .folders,
        FolderChoice::None
    );
    assert!(stored.exec_follow_ups.is_empty());
    assert!(stored.pending_rights.is_empty());
    assert_eq!(
        grant.state,
        DirectGrantState::Accepted,
        "the connection stays"
    );
}

#[test]
fn abgleich_task_narrowing_step_goes_to_the_worker_first_and_a_widening_does_not() {
    let (mut everything, target) = host();
    confirm(&mut everything, &target, &RightsPreset::All.rights());
    // Desktop view and control go, commands and terminal stay.
    let narrowing = entry_for(&everything, &target, &RightsPreset::FilesExec.rights());
    assert_eq!(narrowing.phase, JournalPhase::PendingApply);
    assert!(narrowing
        .mutation
        .only_narrows(&mut everything.clone(), &identity()));

    // A grant that allows nothing is the plain revocation order already.
    let revoke = entry_for(&everything, &target, &RightsPreset::None.rights());
    assert!(!revoke.mutation.policy.grants_any());

    let (mut files_exec, _) = host();
    confirm(&mut files_exec, &target, &RightsPreset::FilesExec.rights());
    let widening = entry_for(&files_exec, &target, &RightsPreset::All.rights());
    assert!(!widening
        .mutation
        .only_narrows(&mut files_exec.clone(), &identity()));

    // Without a running worker there is nothing to narrow, nothing fails.
    apply_narrowing_first(None, &narrowing, &everything, &identity()).unwrap();
}
