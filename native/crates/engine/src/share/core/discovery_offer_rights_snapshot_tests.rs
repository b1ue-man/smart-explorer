//! Plan 2026-10-31 F5: the daemon's offer list names the rights of every
//! running PIN offer, so each surface shows what was really given.
use super::{DiscoveryOfferBook, OwnDiscoveryOffer};
use crate::share::connection_rights::{ConnectionRights, RightsPreset};
use crate::share::{DiscoveryEvent, DiscoveryOfferStopReason, DiscoveryPublishTarget};

fn prepared(offer_id: &str, until: i64) -> DiscoveryEvent {
    DiscoveryEvent::OfferPrepared {
        offer_id: offer_id.into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: until,
    }
}

fn published(offer_id: &str, until: i64) -> DiscoveryEvent {
    DiscoveryEvent::OfferPublished {
        offer_id: offer_id.into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: until,
    }
}

fn rights_event(offer_id: &str, rights: Option<ConnectionRights>) -> DiscoveryEvent {
    DiscoveryEvent::OfferRights {
        offer_id: offer_id.into(),
        rights,
    }
}

#[test]
fn abgleich_task_offer_book_names_the_rights_of_a_running_offer() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_offer_book_names_the_rights_of_a_running_offer",
            abgleich_task_offer_book_names_the_rights_of_a_running_offer_case,
        ),
        (
            "abgleich_task_offer_book_offer_without_rights_waits_and_other_offers_stay_apart",
            abgleich_task_offer_book_offer_without_rights_waits_and_other_offers_stay_apart,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn abgleich_task_offer_book_names_the_rights_of_a_running_offer_case() {
    let mut book = DiscoveryOfferBook::default();
    book.observe(&prepared("a", 1_300));
    book.observe(&rights_event("a", Some(RightsPreset::FilesExec.rights())));
    let offers = book.offers(1_000);
    assert_eq!(offers[0].rights, Some(RightsPreset::FilesExec.rights()));

    // The lease confirmation, a lapse and the renewal rewrite the offer; the
    // rights stay exactly as announced.
    book.observe(&published("a", 1_300));
    assert_eq!(
        book.offers(1_000)[0].rights,
        Some(RightsPreset::FilesExec.rights())
    );
    book.observe(&prepared("a", 1_300));
    assert_eq!(
        book.offer_for_target(&DiscoveryPublishTarget::Direct, 1_000)
            .and_then(|offer| offer.rights.clone()),
        Some(RightsPreset::FilesExec.rights())
    );
}

fn abgleich_task_offer_book_offer_without_rights_waits_and_other_offers_stay_apart() {
    let mut book = DiscoveryOfferBook::default();
    book.observe(&prepared("none", 1_300));
    book.observe(&rights_event("none", None));
    book.observe(&prepared("ro", 1_400));
    book.observe(&rights_event("ro", Some(RightsPreset::ReadOnly.rights())));
    let offers = book.offers(1_000);
    assert_eq!(offers[0].offer_id, "none");
    assert_eq!(offers[0].rights, None);
    assert_eq!(offers[1].offer_id, "ro");
    assert_eq!(offers[1].rights, Some(RightsPreset::ReadOnly.rights()));
    // Cleared again by an announcement without rights.
    book.observe(&rights_event("ro", None));
    assert_eq!(book.offers(1_000)[1].rights, None);
}

#[test]
fn abgleich_task_offer_book_rights_follow_the_offer_to_its_end() {
    let mut book = DiscoveryOfferBook::default();
    // Announced before the offer was seen: kept until it is.
    book.observe(&rights_event("early", Some(RightsPreset::All.rights())));
    book.observe(&published("early", 1_300));
    assert_eq!(
        book.offers(1_000)[0].rights,
        Some(RightsPreset::All.rights())
    );

    book.observe(&DiscoveryEvent::OfferStopped {
        offer_id: "early".into(),
        reason: DiscoveryOfferStopReason::Paired,
    });
    // A later offer starts without the rights of the ended one.
    book.observe(&prepared("later", 1_300));
    assert_eq!(book.offers(1_000)[0].offer_id, "later");
    assert_eq!(book.offers(1_000)[0].rights, None);

    // Rights announced for an offer that ended meanwhile do not linger.
    book.observe(&rights_event("gone", Some(RightsPreset::All.rights())));
    book.observe(&DiscoveryEvent::OfferStopped {
        offer_id: "gone".into(),
        reason: DiscoveryOfferStopReason::Requested,
    });
    book.observe(&prepared("gone", 1_300));
    let gone = book
        .offers(1_000)
        .into_iter()
        .find(|offer| offer.offer_id == "gone")
        .expect("offer present");
    assert_eq!(gone.rights, None);
}

#[test]
fn abgleich_task_offer_snapshot_keeps_the_wire_form_compatible() {
    let with_rights = OwnDiscoveryOffer {
        offer_id: "a".into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: 42,
        published: true,
        rights: Some(RightsPreset::ReadWrite.rights()),
        renewed_after_failed_attempts: false,
        pin: None,
    };
    let json = serde_json::to_string(&with_rights).unwrap();
    let back: OwnDiscoveryOffer = serde_json::from_str(&json).unwrap();
    assert_eq!(back, with_rights);

    // A snapshot of an older daemon has no rights: the offer then waits.
    let older = serde_json::json!({
        "offer_id": "a",
        "target": "Direct",
        "display_alias": "Laptop",
        "discoverable_until": 42,
        "published": false,
    });
    let decoded: OwnDiscoveryOffer = serde_json::from_value(older).unwrap();
    assert_eq!(decoded.rights, None);

    // Without rights the field is not written (older clients read the same).
    let without = OwnDiscoveryOffer {
        rights: None,
        ..with_rights
    };
    assert!(!serde_json::to_string(&without).unwrap().contains("rights"));
}
