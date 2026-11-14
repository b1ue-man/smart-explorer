//! cr-F2d (E17 M21, Plan 11.5 and 8.3): renewed relation data are taken over
//! only for the one accepted contact pinned to exactly this device, and a
//! removed device cannot renew its way back in. The profile file lives in an
//! isolated data folder; the refusal path reads and writes no credential.
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use super::*;
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::direct_reciprocal::DirectRelationMaterial;
use crate::share::types::{DirectAccessState, DirectContact};

static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// The app data folder of this program in a new directory for one test. The
/// variable that names the data home is put back when the test ends, also on
/// a panic.
struct IsolatedDataFolder {
    _lock: MutexGuard<'static, ()>,
    root: tempfile::TempDir,
    key: &'static str,
    saved: Option<OsString>,
}

impl IsolatedDataFolder {
    fn new() -> Self {
        let lock = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = if cfg!(windows) {
            "APPDATA"
        } else {
            "XDG_DATA_HOME"
        };
        let saved = std::env::var_os(key);
        let root = tempfile::tempdir().expect("a temporary data folder");
        std::env::set_var(key, root.path());
        Self {
            _lock: lock,
            root,
            key,
            saved,
        }
    }

    fn home(&self) -> Option<String> {
        Some(self.root.path().to_string_lossy().into_owned())
    }
}

impl Drop for IsolatedDataFolder {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn identity_of(device: &str, seed: u8) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(device, device, &iroh::SecretKey::from_bytes(&[seed; 32]))
}

fn peer_of(identity: &DirectPeerIdentity, lookup_id: &str, secret: u8) -> DirectReciprocalPeer {
    DirectReciprocalPeer::authenticated(
        identity.clone(),
        DirectRelationMaterial::new(lookup_id, vec![secret; 32]).expect("relation material"),
    )
    .expect("authenticated peer")
}

/// The profiles of a user who paired `old`: one accepted contact `contact-l22`.
fn paired_profiles(old: &DirectReciprocalPeer) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    let applied = profiles
        .apply_reciprocal_direct_peer(old, "contact-l22", 100, PairingOrigin::UserPairing)
        .expect("pairing");
    assert_eq!(
        applied,
        DirectReciprocalApply::Changed {
            contact_id: "contact-l22".into()
        }
    );
    profiles
}

/// A copy of `profiles` whose first contact is changed by `change`.
fn edited(profiles: &ShareProfiles, change: impl FnOnce(&mut DirectContact)) -> ShareProfiles {
    let mut edited = profiles.clone();
    change(&mut edited.direct_contacts[0]);
    edited
}

#[test]
fn code_renewal_task_renewable_contact_is_the_one_accepted_contact_of_that_exact_device() {
    let host = identity_of("l22-device-a", 201);
    let old = peer_of(&host, "lookup-l22-old", 11);
    let renewed = peer_of(&host, "lookup-l22-new", 12);
    let accepted = paired_profiles(&old);
    assert_eq!(
        renewable_contact(&accepted, &renewed).map(|contact| contact.id.as_str()),
        Some("contact-l22")
    );

    let other_node = iroh::SecretKey::from_bytes(&[203; 32]).public().to_string();
    let other_key = iroh::SecretKey::from_bytes(&[204; 32]).public().to_string();
    let mut twice = accepted.clone();
    let mut second = twice.direct_contacts[0].clone();
    second.id = "contact-l22-second".into();
    second.lookup_id = "lookup-l22-third".into();
    twice.direct_contacts.push(second);
    let refused = [
        (
            "another fingerprint",
            edited(&accepted, |contact| {
                contact.expected_fingerprint = "fingerprint-of-another-device".into()
            }),
        ),
        (
            "another expected node",
            edited(&accepted, |contact| {
                contact.expected_node_id = other_node.clone()
            }),
        ),
        (
            "another remote key",
            edited(&accepted, |contact| {
                contact.remote_public_key = Some(other_key.clone())
            }),
        ),
        (
            "access not accepted",
            edited(&accepted, |contact| {
                contact.access_state = DirectAccessState::Ignored
            }),
        ),
        (
            "the same lookup id",
            edited(&accepted, |contact| {
                contact.lookup_id = "lookup-l22-new".into()
            }),
        ),
        ("two contacts of one device", twice),
    ];
    for (case, profiles) in &refused {
        assert!(
            renewable_contact(profiles, &renewed).is_none(),
            "{case}: the renewal must not be taken over"
        );
    }
}

#[test]
fn code_renewal_task_renewal_refuses_a_removed_device_and_keeps_the_profile() {
    let folder = IsolatedDataFolder::new();
    let home = folder.home();
    let host = identity_of("l22-device-b", 205);
    let old = peer_of(&host, "lookup-l22b-old", 13);
    let renewed = peer_of(&host, "lookup-l22b-new", 14);
    let mut profiles = paired_profiles(&old);
    profiles.record_removed_direct_peer(&host, 200);
    profiles
        .save()
        .expect("the profile is saved in the isolated folder");

    let before = ShareProfiles::load_checked(home.clone()).expect("the saved profile loads");
    // The contact would be renewable; only the removal refuses the renewal.
    assert!(renewable_contact(&before, &renewed).is_some());
    assert!(before.removed_direct_peer(renewed.identity()).is_some());
    let before_json = serde_json::to_value(&before).expect("profile as JSON");

    let expected = SecretString::encoded(renewed.material().secret());
    assert_eq!(renew_contact_material(&home, &renewed, &expected), Ok(()));

    let after = ShareProfiles::load_checked(home).expect("the profile still loads");
    assert_eq!(
        serde_json::to_value(&after).expect("profile as JSON"),
        before_json
    );
}

/// E43, M21, S2 (L2 point 4): „Gerät entfernen“ runs
/// `forget_direct_peer_persisted` (Desktop, Android, `se setup`, `se share
/// grants`); the machine login target the removed device had sent goes with
/// it, in the same isolated data folder as the profile.
#[test]
fn login_ziel_task_forget_direct_peer_persisted_deletes_the_target() {
    use crate::login::core::{gateway_types::GatewayTarget, model::VERSION};
    use crate::share::login_transport::{
        paired_login_target_bound, target_store, target_wire, SessionPeer,
    };

    let folder = IsolatedDataFolder::new();
    let home = folder.home();
    let host = identity_of("l2-forget-device", 207);
    let old = peer_of(&host, "lookup-l2-forget", 15);
    let mut profiles = paired_profiles(&old);
    assert_eq!(
        profiles.direct_contacts[0].remote_device_id.as_deref(),
        Some("l2-forget-device")
    );
    profiles
        .save()
        .expect("the profile is saved in the isolated folder");

    // The host's answer as the viewer took it into its book.
    let gateway = GatewayTarget {
        format: VERSION,
        machine_id: "machine-l2-forget".into(),
        public_key: iroh::SecretKey::from_bytes(&[208; 32]).public().to_string(),
        direct_addresses: vec!["192.0.2.21:4433".parse().expect("address")],
        relay_urls: vec!["https://relay.example.test/".into()],
        use_default_relays: false,
        allow_plaintext_relays: false,
    };
    let grants = vec!["l2-forget-viewer".to_string()];
    let message = target_wire::host_message(&target_wire::HostFacts {
        host_device_id: "l2-forget-device",
        device_id: "l2-forget-viewer",
        steering: true,
        account: Some("1000"),
        own_grants: &grants,
        target: Some(&gateway),
    });
    let peer = SessionPeer {
        device_id: "l2-forget-device".into(),
        public_key: host.public_key.clone(),
        node_id: host.node_id.clone(),
        device_name: "Gerät l2".into(),
        relation_id: "contact-l22".into(),
        pinned: true,
    };
    let stored = target_store::update(|book| {
        let change = book.apply(&peer, &message, 300);
        let changed = change.is_ok();
        (change, changed)
    })
    .expect("the book is written");
    assert_eq!(stored, Ok(target_store::Change::Stored));
    assert!(paired_login_target_bound("l2-forget-device", &host.public_key).is_some());

    let (_, change, forgotten) =
        ShareProfiles::forget_direct_peer_persisted(home.clone(), "contact-l22")
            .expect("the contact is removed");
    assert!(change.changed && forgotten.is_some());
    assert!(paired_login_target_bound("l2-forget-device", &host.public_key).is_none());
    let book = target_store::load().expect("the book loads");
    assert!(book
        .targets()
        .iter()
        .all(|target| target.device_id != "l2-forget-device"));
    drop(folder);
}
