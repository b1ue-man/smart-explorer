//! Plan 2026-11-06 R1/R2: a PIN pairing, „Eigene Freigaben öffnen“ and the
//! repair take the rights the user set for new connections; a new key of a
//! known device never does; any deliberate pairing confirms a grant waiting
//! for re-confirmation; one-way pairing still opens nothing new.
use super::*;
use crate::share::relation_rights::SessionAuthorization;
use crate::share::{
    DirectGrantState, DirectPeerIdentity, DirectReciprocalPeer, DirectRelationMaterial,
    PairingOrigin, ShareProfiles, SharedRoot, DIRECT_EXPORT_SCOPE,
};

const NOW: i64 = 100;

fn profiles(code_rights: Option<ConnectionRights>) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.default_direct_exports.roots = vec![SharedRoot::new_in_scope(
        "Docs",
        "/home/u/Docs",
        DIRECT_EXPORT_SCOPE,
    )];
    profiles.set_code_rights(code_rights).unwrap();
    profiles
}

fn peer() -> DirectReciprocalPeer {
    DirectReciprocalPeer::authenticated(
        DirectPeerIdentity::from_secret("pin", "Pin", &iroh::SecretKey::from_bytes(&[9; 32])),
        DirectRelationMaterial::new("remote-lookup", vec![8; 32]).unwrap(),
    )
    .unwrap()
}

fn visible(profiles: &ShareProfiles) -> usize {
    let grant = profiles.grant_for("pin").expect("grant");
    SessionAuthorization::direct(profiles.default_direct_exports.clone(), grant)
        .exports
        .roots
        .len()
}

#[test]
fn rechte_task_pairing_takes_the_rights_of_new_connections() {
    let mut with = profiles(Some(RightsPreset::ReadWrite.rights()));
    with.apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairing)
        .unwrap();
    assert!(
        with.pending_rights.is_empty(),
        "the set rights apply at once"
    );
    assert!(with.grant_for("pin").unwrap().write);
    assert_eq!(visible(&with), 1, "the paired device sees the folder");

    // Without such rights the grant waits, as before (F1).
    let mut without = profiles(None);
    without
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairing)
        .unwrap();
    assert_eq!(without.pending_rights.len(), 1);
    assert_eq!(visible(&without), 0);
}

#[test]
fn rechte_task_one_way_pairing_opens_nothing_but_reconfirms() {
    let mut profiles = profiles(Some(RightsPreset::All.rights()));
    profiles
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairingOneWay)
        .unwrap();
    assert!(
        profiles.direct_grants.is_empty(),
        "one way creates no grant"
    );
    // A grant suspended after this device's code rotation: a deliberate
    // pairing, one way included, confirms it again (Exec stays off).
    profiles
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairing)
        .unwrap();
    profiles.direct_grants[0].state = DirectGrantState::Reconfirm;
    profiles
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::AutomaticRepair)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Reconfirm);
    profiles
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairingOneWay)
        .unwrap();
    assert_eq!(profiles.direct_grants[0].state, DirectGrantState::Accepted);
    assert!(!profiles.direct_grants[0].exec.enabled);
}

#[test]
fn rechte_task_new_key_never_takes_the_rights_of_new_connections() {
    let mut profiles = profiles(Some(RightsPreset::All.rights()));
    profiles
        .apply_reciprocal_direct_peer(&peer(), "c-pin", NOW, PairingOrigin::UserPairing)
        .unwrap();
    profiles.settle_new_direct_grant("pin", RightsOrigin::IdentityReplaced, None, NOW);
    assert!(!profiles.grant_for("pin").unwrap().write);
    assert_eq!(visible(&profiles), 0);
    assert_eq!(profiles.pending_rights.len(), 1);
}
