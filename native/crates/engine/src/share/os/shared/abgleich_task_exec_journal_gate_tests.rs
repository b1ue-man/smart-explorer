//! Review 2, follow-up of W8 (2026-10-31): an acceptance that may give or
//! change commands, terminal or screen fails as a whole, before anything is
//! stored, when the Exec-grant journal cannot be used (identity not fully
//! pinned; the background worker of this version is checked by
//! `ensure_worker_ready`), and an acceptance that touches none of those
//! rights needs neither the worker nor the check.
use super::{decision_failure, exec_open_text, require_exec_journal, CONNECTION_STORED};
use crate::share::connection_rights::{ConnectionRights, RightsPreset, RightsTarget};
use crate::share::exec_follow_up::FollowUpEffect;
use crate::share::{DirectGrant, DirectGrantState, DirectPeerIdentity, ExecGrant, ShareProfiles};

fn requester() -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret("peer", "Peer", &iroh::SecretKey::from_bytes(&[5; 32]))
}

fn unpinned() -> DirectPeerIdentity {
    DirectPeerIdentity {
        node_id: String::new(),
        ..requester()
    }
}

fn with_exec_grant() -> ShareProfiles {
    let peer = requester();
    let mut profiles = ShareProfiles::default();
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
        .unwrap();
    profiles
}

#[test]
fn abgleich_task_gate_files_only_acceptances_need_no_worker() {
    let mut profiles = ShareProfiles::default();
    let read_only = RightsPreset::ReadOnly.rights();
    // Even an unpinned identity is no problem for folders and write.
    for identity in [requester(), unpinned()] {
        assert_eq!(
            require_exec_journal(&profiles, &identity, Some(&read_only)),
            Ok(false)
        );
        assert_eq!(require_exec_journal(&profiles, &identity, None), Ok(false));
    }
    profiles.code_rights = Some(RightsPreset::ReadWrite.rights());
    assert_eq!(
        require_exec_journal(&profiles, &unpinned(), None),
        Ok(false),
        "the Direct code's rights carry no commands"
    );
}

#[test]
fn abgleich_task_gate_exec_rights_without_a_pinned_identity_fail_the_whole_acceptance() {
    let mut profiles = ShareProfiles::default();
    let chosen = RightsPreset::FilesExec.rights();
    let error = require_exec_journal(&profiles, &unpinned(), Some(&chosen)).unwrap_err();
    assert!(error.contains("gepinnte Identitaet"), "{error}");
    assert!(error.contains("unveraendert"), "{error}");
    // The rights of the Direct code count when none were chosen.
    profiles.code_rights = Some(RightsPreset::All.rights());
    assert!(require_exec_journal(&profiles, &unpinned(), None).is_err());
    // Every part of the menu counts, also the screen alone.
    let screen = ConnectionRights {
        screen_view: crate::share::access_grant::CapabilityDecision::Allow,
        ..RightsPreset::ReadOnly.rights()
    };
    assert!(require_exec_journal(&ShareProfiles::default(), &unpinned(), Some(&screen)).is_err());
}

#[test]
fn abgleich_task_gate_a_narrower_choice_for_a_grant_with_exec_goes_through_the_journal_too() {
    let profiles = with_exec_grant();
    let none = RightsPreset::None.rights();
    assert!(
        require_exec_journal(&profiles, &unpinned(), Some(&none)).is_err(),
        "revoking a running grant is a journal entry as well"
    );
    // Without a choice the existing grant is not touched by the gate.
    assert_eq!(
        require_exec_journal(&profiles, &unpinned(), None),
        Ok(false)
    );
}

#[test]
fn abgleich_task_gate_a_failure_after_storing_says_so() {
    let stored = format!("{CONNECTION_STORED}Befehle noch nicht freigegeben");
    assert_eq!(decision_failure(&stored, "nicht gespeichert"), stored);
    assert_eq!(
        decision_failure(
            "Fingerabdruck falsch",
            "Direkt-Entscheidung nicht gespeichert"
        ),
        "Direkt-Entscheidung nicht gespeichert: Fingerabdruck falsch"
    );
}

/// G3 (review 3): the message of a failed journal step says whether rights
/// are still to be given or still to be taken away.
#[test]
fn abgleich_task_gate_message_tells_not_yet_revoked_from_not_yet_released() {
    let narrows = exec_open_text(FollowUpEffect {
        narrows: true,
        widens: false,
    });
    assert!(narrows.contains("noch nicht entzogen"), "{narrows}");
    assert!(!narrows.contains("noch nicht freigegeben"), "{narrows}");
    assert!(narrows.contains("Rechte offen"), "{narrows}");

    let widens = exec_open_text(FollowUpEffect {
        narrows: false,
        widens: true,
    });
    assert!(widens.contains("noch nicht freigegeben"), "{widens}");
    assert!(!widens.contains("entzogen"), "{widens}");
    assert_eq!(widens, exec_open_text(FollowUpEffect::default()));

    let both = exec_open_text(FollowUpEffect {
        narrows: true,
        widens: true,
    });
    assert!(both.contains("noch nicht entzogen"), "{both}");
    assert!(both.contains("noch nicht freigegeben"), "{both}");
    // The text after storing is shown unchanged by every surface.
    let stored = format!("{CONNECTION_STORED}Befehle, Terminal oder Bildschirm {narrows}: x");
    assert_eq!(decision_failure(&stored, "nicht gespeichert"), stored);
}
