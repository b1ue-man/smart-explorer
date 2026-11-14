//! Plan 2026-10-31 F5: `share.status` names the rights of the running PIN
//! offer, so Android shows what was really given after a restart.
use super::*;
use crate::share::connection_rights::RightsPreset;
use crate::share::discovery_state::{DiscoveryOfferPhase, DiscoveryUiState};

fn discovery_of(state: &DiscoveryUiState) -> Value {
    let profiles = ShareProfiles::default();
    let exec_provider = ExecProviderStatus {
        available: true,
        provider: "android-subreaper".into(),
        detail: String::new(),
        elevated: false,
        user_label: "Android-App".into(),
    };
    let notices = VecDeque::new();
    let aliases = BTreeMap::new();
    discovery_json(&StatusInput {
        worker: None,
        profiles: &profiles,
        identity: None,
        server: "",
        poll_error: None,
        discovery: state,
        offer_aliases: &aliases,
        last_exchange: None,
        notices: &notices,
        now_secs: 1_700_000_000,
        exec_provider: &exec_provider,
    })
}

#[test]
fn abgleich_task_status_offer_carries_its_rights() {
    let mut state = DiscoveryUiState::default();
    state.offer_updated(
        "o1".into(),
        DiscoveryPublishTarget::Direct,
        1_700_000_300,
        DiscoveryOfferPhase::Published,
    );
    // No rights announced: the device waits (null).
    let waiting = discovery_of(&state);
    assert_eq!(waiting["offer"]["offerId"], "o1");
    assert!(waiting["offer"]["rights"].is_null());

    state.set_offer_rights("o1", Some(RightsPreset::FilesExec.rights()));
    let given = discovery_of(&state);
    let rights = &given["offer"]["rights"];
    assert_eq!(rights["preset"], "files-exec");
    assert_eq!(rights["summary"], "Dateien und Codeausführung");
    assert_eq!(rights["write"], true);
    assert_eq!(rights["commands"], "allow");
    assert_eq!(rights["terminal"], "allow");
    assert_eq!(rights["screenView"], "off");
    assert_eq!(rights["folders"]["kind"], "all");
}

/// E49 on Android: an offer that ended after too many failed attempts waits in
/// its pause; `share.status` names the offer to stop („Nicht erneuern“) and
/// when the offer with a new PIN goes out, and nothing once it is stopped.
#[test]
fn pin_neu_task_android_status_names_the_waiting_renewal() {
    let mut state = DiscoveryUiState::default();
    assert!(discovery_of(&state)["renewalWaiting"].is_null());
    let waiting = crate::share::WaitingRenewal {
        replaces: "ersetzt".into(),
        target: crate::share::DiscoveryPublishTarget::Direct,
        display_alias: "Pixel".into(),
        due: 1_700_000_030,
    };
    state.adopt_live(&[], std::slice::from_ref(&waiting));
    let shown = discovery_of(&state);
    assert_eq!(shown["renewalWaiting"]["replaces"], "ersetzt");
    assert_eq!(shown["renewalWaiting"]["alias"], "Pixel");
    assert_eq!(shown["renewalWaiting"]["dueMs"], 1_700_000_030_000_i64);
    assert_eq!(
        shown["renewalWaiting"]["hint"],
        crate::share::discovery_state::pin_renewal_hint()
    );
    assert!(shown["offer"].is_null());
    // The stop was taken: the next snapshot carries the renewal no more.
    state.adopt_live(&[], &[]);
    assert!(discovery_of(&state)["renewalWaiting"].is_null());
}
