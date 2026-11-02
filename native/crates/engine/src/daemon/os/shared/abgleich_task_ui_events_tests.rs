//! G7 (review 2026-10-31): the rights of a running PIN offer never enter the
//! backlog the GUI clients poll, so an older GUI of a handover keeps
//! decoding it; the daemon's offer book still names them for the snapshot.
use super::{push, reaches_clients};
use crate::share::connection_rights::RightsPreset;
use crate::share::{DiscoveryEvent, DiscoveryOfferBook, DiscoveryPublishTarget, ShareEvent};

fn rights_event() -> ShareEvent {
    ShareEvent::Discovery(DiscoveryEvent::OfferRights {
        offer_id: "offer".into(),
        rights: Some(RightsPreset::All.rights()),
    })
}

fn prepared() -> DiscoveryEvent {
    DiscoveryEvent::OfferPrepared {
        offer_id: "offer".into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: 1_300,
    }
}

#[test]
fn abgleich_task_offer_rights_stay_out_of_the_gui_backlog() {
    assert!(!reaches_clients(&rights_event()));
    assert!(reaches_clients(&ShareEvent::Discovery(prepared())));
    let mut events = Vec::new();
    push(&mut events, ShareEvent::Discovery(prepared()));
    push(&mut events, rights_event());
    push(&mut events, ShareEvent::Status("after".into()));
    assert_eq!(events.len(), 2);
    // The backlog an older GUI decodes holds no variant it does not know.
    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("OfferRights"), "{encoded}");
    assert!(encoded.contains("OfferPrepared"));
}

#[test]
fn abgleich_task_offer_rights_still_reach_the_daemon_offer_book() {
    let mut book = DiscoveryOfferBook::default();
    book.observe(&prepared());
    if let ShareEvent::Discovery(event) = rights_event() {
        book.observe(&event);
    }
    let offers = book.offers(1_000);
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].rights, Some(RightsPreset::All.rights()));
    // The snapshot form carries them as an optional field older clients
    // skip as unknown.
    let encoded = serde_json::to_value(&offers[0]).unwrap();
    assert!(encoded.get("rights").is_some());
}
