//! Plan 2026-11-08 E49 (security line S9): an offer that ended after too many
//! failed attempts is offered again after a pause with a new PIN, only then,
//! and never beyond the end the person chose. Time is passed as an argument.
use super::{DiscoveryOfferBook, PIN_RENEWAL_PAUSE_SECS};
use crate::share::connection_rights::{ConnectionRights, RightsPreset};
use crate::share::{
    discovery_pin_strength, DiscoveryCommand, DiscoveryEvent, DiscoveryOfferStopReason,
    DiscoveryPublishTarget,
};

const TOO_MANY: DiscoveryOfferStopReason = DiscoveryOfferStopReason::TooManyFailedAttempts;

fn room(id: &str) -> DiscoveryPublishTarget {
    DiscoveryPublishTarget::Room {
        room_profile_id: id.into(),
    }
}

/// A published offer for `target`, as the worker's events report it.
fn start(
    book: &mut DiscoveryOfferBook,
    offer_id: &str,
    target: DiscoveryPublishTarget,
    until: i64,
    rights: Option<ConnectionRights>,
    now: i64,
) {
    for event in [
        DiscoveryEvent::OfferPrepared {
            offer_id: offer_id.into(),
            target: target.clone(),
            display_alias: "Laptop".into(),
            discoverable_until: until,
        },
        DiscoveryEvent::OfferRights {
            offer_id: offer_id.into(),
            rights,
        },
        DiscoveryEvent::OfferPublished {
            offer_id: offer_id.into(),
            target,
            display_alias: "Laptop".into(),
            discoverable_until: until,
        },
    ] {
        book.observe_at(&event, now);
    }
}

fn stopped(offer_id: &str, reason: DiscoveryOfferStopReason) -> DiscoveryEvent {
    DiscoveryEvent::OfferStopped {
        offer_id: offer_id.into(),
        reason,
    }
}

#[test]
fn pin_neu_task_fifth_failure_renews_after_30_seconds_with_new_pin() {
    let mut book = DiscoveryOfferBook::default();
    let rights = RightsPreset::ReadWrite.rights();
    start(
        &mut book,
        "first",
        DiscoveryPublishTarget::Direct,
        2_800,
        Some(rights.clone()),
        1_000,
    );
    // The fifth failed attempt ends the offer at 1_100.
    book.observe_at(&stopped("first", TOO_MANY), 1_100);
    assert!(book.offers(1_100).is_empty());
    assert_eq!(book.pending_renewals(), 1);

    // Nothing is due during the pause, and the renewal is due right after it.
    let due_at = 1_100 + PIN_RENEWAL_PAUSE_SECS;
    assert_eq!(PIN_RENEWAL_PAUSE_SECS, 30);
    assert!(book.claim_due_renewals(due_at - 1).is_empty());
    let due = book.claim_due_renewals(due_at);
    assert_eq!(due.len(), 1);
    // A claimed renewal is not handed out a second time.
    assert!(book.claim_due_renewals(due_at).is_empty());

    let renewal = &due[0];
    assert_eq!(renewal.replaces, "first");
    assert_eq!(renewal.target, DiscoveryPublishTarget::Direct);
    assert_eq!(renewal.display_alias, "Laptop");
    assert_eq!(renewal.rights, Some(rights.clone()));

    // The publish command: target, name and rights of the replaced offer,
    // the remaining time, and a generated PIN without the weak-PIN opt-in.
    let pin = DiscoveryOfferBook::fresh_renewal_pin().expect("random PIN");
    let command = renewal
        .publish_command(pin.clone(), due_at)
        .expect("time remains");
    match command {
        DiscoveryCommand::Publish {
            target,
            display_alias,
            pin: sent,
            duration_secs,
            allow_weak_pin,
            rights: sent_rights,
        } => {
            assert_eq!(target, DiscoveryPublishTarget::Direct);
            assert_eq!(display_alias, "Laptop");
            assert_eq!(duration_secs, 2_800 - 1_130);
            assert!(!allow_weak_pin);
            assert_eq!(sent_rights, Some(rights.clone()));
            assert_eq!(sent.as_bytes().len(), 6);
            assert!(discovery_pin_strength(sent.as_bytes()).is_acceptable());
            assert_eq!(sent, pin);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    // The worker reports the new offer; the daemon marks it with the new PIN.
    start(
        &mut book,
        "second",
        DiscoveryPublishTarget::Direct,
        2_800,
        Some(rights.clone()),
        due_at,
    );
    assert!(book.finish_renewal("first"));
    assert!(book.mark_renewed("second", pin.clone()));
    assert_eq!(book.pending_renewals(), 0);
    let offer = book.offers(due_at).pop().expect("renewed offer");
    assert!(offer.renewed_after_failed_attempts);
    assert_eq!(offer.pin, Some(pin.clone()));
    assert_eq!(offer.rights, Some(rights));

    // A lease renewal rewrites the offer; mark and PIN stay. The PIN shows in
    // no Debug output and travels in the serialized snapshot only.
    book.observe_at(
        &DiscoveryEvent::OfferPublished {
            offer_id: "second".into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: 2_800,
        },
        due_at + 5,
    );
    let offer = book.offers(due_at).pop().expect("renewed offer");
    assert!(offer.renewed_after_failed_attempts);
    let text = String::from_utf8(pin.as_bytes().to_vec()).expect("digits");
    assert!(!format!("{offer:?}").contains(&text));
    let json = serde_json::to_value(&offer).expect("serializes");
    assert_eq!(json["pin"], text.as_str());
    assert_eq!(json["renewed_after_failed_attempts"], true);

    // The hint carries the number of attempts from the same constant, and the
    // interface state takes the hint and the new PIN from the offer list over
    // the one call the desktop and Android make (`retain_live_offers`).
    let hint = crate::share::discovery_state::pin_renewal_hint();
    assert_eq!(
        hint,
        "5 falsche PIN-Versuche – vermutlich Rateversuche; neue PIN"
    );
    let mut state = crate::share::discovery_state::DiscoveryUiState::default();
    state.retain_live_offers(std::slice::from_ref(&offer));
    let shown = state.active_offers[0]
        .renewal_notice()
        .expect("retain_live_offers copies the renewal (adopt_live_renewal)");
    assert_eq!(shown.hint(), hint);
    assert_eq!(shown.pin_text().as_deref(), Some(text.as_str()));
    assert_eq!(offer.renewal_notice(), Some(shown));
}

#[test]
fn pin_neu_task_person_stop_creates_no_renewal() {
    let mut book = DiscoveryOfferBook::default();
    // The person stops a running offer: nothing follows.
    start(&mut book, "a", room("r1"), 5_000, None, 1_000);
    book.observe_at(&stopped("a", DiscoveryOfferStopReason::Requested), 1_100);
    assert_eq!(book.pending_renewals(), 0);

    // The person stops the replaced offer during the pause: the renewal goes.
    start(&mut book, "b", room("r2"), 5_000, None, 1_000);
    book.observe_at(&stopped("b", TOO_MANY), 1_100);
    assert!(book.renewal_pending("b"));
    assert!(book.cancel_renewal("b"));
    assert!(!book.renewal_pending("b"));
    assert!(book.claim_due_renewals(4_000).is_empty());
    assert!(!book.cancel_renewal("b"));

    // The same through the worker's stop event of the replaced id.
    start(&mut book, "c", room("r3"), 5_000, None, 1_000);
    book.observe_at(&stopped("c", TOO_MANY), 1_100);
    book.observe_at(&stopped("c", DiscoveryOfferStopReason::Requested), 1_110);
    assert_eq!(book.pending_renewals(), 0);

    // A stop while the daemon publishes: the claimed renewal reports it was
    // cancelled, so the daemon withdraws what it just published.
    start(&mut book, "d", room("r4"), 5_000, None, 1_000);
    book.observe_at(&stopped("d", TOO_MANY), 1_100);
    assert_eq!(book.claim_due_renewals(1_130).len(), 1);
    assert!(book.cancel_renewal("d"));
    assert!(!book.finish_renewal("d"));

    // A stopped Share worker holds no offers, so nothing renews either.
    start(&mut book, "e", room("r5"), 5_000, None, 1_000);
    book.observe_at(&stopped("e", TOO_MANY), 1_100);
    assert_eq!(book.pending_renewals(), 1);
    book.end_all(DiscoveryOfferStopReason::WorkerStopped);
    assert_eq!(book.pending_renewals(), 0);
}

#[test]
fn pin_neu_task_paired_offer_creates_no_renewal() {
    // Only too many failed attempts renew; every other end leaves nothing.
    for (index, reason) in [
        DiscoveryOfferStopReason::Requested,
        DiscoveryOfferStopReason::Expired,
        DiscoveryOfferStopReason::TargetUnavailable,
        DiscoveryOfferStopReason::CapabilityUnavailable,
        DiscoveryOfferStopReason::TransportError,
        DiscoveryOfferStopReason::WorkerStopped,
        DiscoveryOfferStopReason::Paired,
    ]
    .into_iter()
    .enumerate()
    {
        let mut book = DiscoveryOfferBook::default();
        let offer_id = format!("offer-{index}");
        start(&mut book, &offer_id, room("r"), 5_000, None, 1_000);
        book.observe_at(&stopped(&offer_id, reason), 1_100);
        assert_eq!(book.pending_renewals(), 0, "{reason:?}");
        assert!(book.claim_due_renewals(4_000).is_empty(), "{reason:?}");
    }

    // A paired offer is gone; a late failure event for it plans nothing.
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "paired", room("r"), 5_000, None, 1_000);
    book.observe_at(&stopped("paired", DiscoveryOfferStopReason::Paired), 1_100);
    book.observe_at(&stopped("paired", TOO_MANY), 1_101);
    assert_eq!(book.pending_renewals(), 0);
}

#[test]
fn pin_neu_task_renewal_keeps_original_deadline() {
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "o1", room("r"), 2_000, None, 1_000);
    book.observe_at(&stopped("o1", TOO_MANY), 1_000);
    let first = book.claim_due_renewals(1_030).pop().expect("due");
    assert_eq!(first.discoverable_until, 2_000);
    let pin = DiscoveryOfferBook::fresh_renewal_pin().expect("random PIN");
    let duration = match first.publish_command(pin, 1_030) {
        Some(DiscoveryCommand::Publish { duration_secs, .. }) => duration_secs,
        other => panic!("unexpected command: {other:?}"),
    };
    // Published at 1_030 for the remaining time, it ends at the old end.
    assert_eq!(1_030 + duration as i64, 2_000);

    // The renewed offer fails five attempts again: the next renewal still
    // ends at the original end and gets less time.
    start(&mut book, "o2", room("r"), 2_000, None, 1_030);
    assert!(book.finish_renewal("o1"));
    book.observe_at(&stopped("o2", TOO_MANY), 1_500);
    let second = book.claim_due_renewals(1_530).pop().expect("due again");
    assert_eq!(second.replaces, "o2");
    assert_eq!(second.discoverable_until, 2_000);
    assert_eq!(second.remaining_secs(1_530), 470);

    // No time remains at the end: nothing is planned, nothing can be sent.
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "late", room("r"), 1_600, None, 1_000);
    book.observe_at(&stopped("late", TOO_MANY), 1_600);
    assert_eq!(book.pending_renewals(), 0);
    let other_pin = DiscoveryOfferBook::fresh_renewal_pin().expect("random PIN");
    assert!(first.publish_command(other_pin, 2_000).is_none());

    // The time runs out during the pause: the renewal is dropped when due.
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "short", room("r"), 1_020, None, 1_000);
    book.observe_at(&stopped("short", TOO_MANY), 1_000);
    assert_eq!(book.pending_renewals(), 1);
    assert!(book.claim_due_renewals(1_030).is_empty());
    assert_eq!(book.pending_renewals(), 0);

    // The person published the target again during the pause: that offer
    // stays, the renewal is dropped.
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "old", room("r"), 5_000, None, 1_000);
    book.observe_at(&stopped("old", TOO_MANY), 1_100);
    start(&mut book, "manual", room("r"), 5_000, None, 1_110);
    assert!(book.claim_due_renewals(1_130).is_empty());
    assert_eq!(book.pending_renewals(), 0);
    assert_eq!(book.offers(1_130).len(), 1);

    // A claimed renewal that failed to publish is tried again later.
    let mut book = DiscoveryOfferBook::default();
    start(&mut book, "retry", room("r"), 5_000, None, 1_000);
    book.observe_at(&stopped("retry", TOO_MANY), 1_100);
    assert_eq!(book.claim_due_renewals(1_130).len(), 1);
    book.postpone_renewal("retry", 1_130);
    assert!(book.claim_due_renewals(1_159).is_empty());
    assert_eq!(book.claim_due_renewals(1_160).len(), 1);
}
