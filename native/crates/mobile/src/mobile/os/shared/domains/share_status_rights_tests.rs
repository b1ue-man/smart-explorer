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
