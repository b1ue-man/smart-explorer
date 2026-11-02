//! Plan 2026-10-31 F5: a restarted interface shows the rights the running
//! PIN offer really carries instead of "wartet ohne Rechte".
use super::super::{DiscoveryOfferPhase, DiscoveryPublishTarget, DiscoveryUiState};
use crate::share::connection_rights::{ConnectionRights, RightsPreset};
use crate::share::discovery_events::apply_share_discovery_event;
use crate::share::{DiscoveryEvent, OwnDiscoveryOffer};

fn live(offer_id: &str, rights: Option<ConnectionRights>) -> OwnDiscoveryOffer {
    OwnDiscoveryOffer {
        offer_id: offer_id.into(),
        target: crate::share::DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: i64::MAX,
        published: true,
        rights,
    }
}

#[test]
fn abgleich_task_restarted_interface_adopts_the_running_offer_with_its_rights() {
    // A fresh interface knows no offer: the daemon's list brings it in.
    let mut state = DiscoveryUiState {
        status: Some("kept".into()),
        ..DiscoveryUiState::default()
    };
    state.retain_live_offers(&[live("o1", Some(RightsPreset::FilesExec.rights()))]);
    let offer = state
        .offer_for_target(&DiscoveryPublishTarget::Direct)
        .expect("offer adopted");
    assert_eq!(offer.offer_id, "o1");
    assert_eq!(offer.phase, DiscoveryOfferPhase::Published);
    assert_eq!(offer.rights, Some(RightsPreset::FilesExec.rights()));
    assert_eq!(offer.alias, "Laptop");
    assert!(offer
        .rights_sentence()
        .ends_with("erhält: Dateien und Codeausführung."));
    // Taking up an offer is no news: the status text stays.
    assert_eq!(state.status.as_deref(), Some("kept"));
    assert!(!state.pending_direct_publish);
}

#[test]
fn abgleich_task_offer_without_rights_says_it_waits() {
    let mut state = DiscoveryUiState::default();
    state.retain_live_offers(&[live("o1", None)]);
    let offer = state
        .offer_for_target(&DiscoveryPublishTarget::Direct)
        .expect("offer adopted");
    assert_eq!(offer.rights, None);
    assert!(offer.rights_sentence().contains("wartet ohne Rechte"));

    // „Keine Rechte“ chosen in advance is a choice, not waiting.
    state.retain_live_offers(&[live("o1", Some(RightsPreset::None.rights()))]);
    let offer = &state.active_offers[0];
    assert!(offer
        .rights_sentence()
        .contains("wird ohne Rechte verbunden"));
}

#[test]
fn abgleich_task_rights_follow_the_event_and_the_snapshot() {
    let mut state = DiscoveryUiState::default();
    apply_share_discovery_event(
        &mut state,
        DiscoveryEvent::OfferPrepared {
            offer_id: "o1".into(),
            target: crate::share::DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: i64::MAX,
        },
    );
    apply_share_discovery_event(
        &mut state,
        DiscoveryEvent::OfferRights {
            offer_id: "o1".into(),
            rights: Some(RightsPreset::ReadOnly.rights()),
        },
    );
    assert_eq!(
        state.active_offers[0].rights,
        Some(RightsPreset::ReadOnly.rights())
    );
    // The snapshot is authoritative: it corrects what an event left.
    state.retain_live_offers(&[live("o1", Some(RightsPreset::All.rights()))]);
    assert_eq!(state.active_offers.len(), 1);
    assert_eq!(
        state.active_offers[0].rights,
        Some(RightsPreset::All.rights())
    );
    // Rights of an offer the interface does not know are ignored.
    state.set_offer_rights("unknown", Some(RightsPreset::All.rights()));
    assert_eq!(state.active_offers.len(), 1);
}

#[test]
fn abgleich_task_a_new_offer_id_for_the_target_drops_the_old_rights() {
    let mut state = DiscoveryUiState::default();
    state.retain_live_offers(&[live("old", Some(RightsPreset::All.rights()))]);
    state.offer_updated(
        "new".into(),
        DiscoveryPublishTarget::Direct,
        i64::MAX,
        DiscoveryOfferPhase::Prepared,
    );
    assert_eq!(state.active_offers.len(), 1);
    assert_eq!(state.active_offers[0].offer_id, "new");
    assert_eq!(state.active_offers[0].rights, None);
}

#[test]
fn abgleich_task_vanished_offers_still_leave_the_interface() {
    let mut state = DiscoveryUiState::default();
    state.retain_live_offers(&[live("o1", None)]);
    state.retain_live_offers(&[]);
    assert!(state.active_offers.is_empty());
    assert_eq!(
        state.status.as_deref(),
        Some(super::super::WORKER_STOPPED_STATUS)
    );
}
