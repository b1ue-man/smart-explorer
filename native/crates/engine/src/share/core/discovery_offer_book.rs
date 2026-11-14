//! The daemon's durable view of this device's own discovery offers. The Share
//! worker reports every offer transition as a `DiscoveryEvent`; folding them
//! here keeps the state readable by every client, independent of who drains
//! the shared UI event buffer.
use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::connection_rights::ConnectionRights;
use super::discovery_signal_types::{
    DiscoveryEvent, DiscoveryOfferStopReason, DiscoveryPin, DiscoveryPublishTarget,
};

#[path = "discovery_pin_renewal.rs"]
mod pin_renewal;
use self::pin_renewal::PendingRenewals;
pub use self::pin_renewal::{PinRenewal, WaitingRenewal, PIN_RENEWAL_PAUSE_SECS};

/// Offers that ended recently, so a publish acknowledged just before its
/// offer stopped still reports the reason.
const RECENTLY_ENDED_CAPACITY: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnDiscoveryOffer {
    pub offer_id: String,
    pub target: DiscoveryPublishTarget,
    pub display_alias: String,
    /// Unix seconds; the offer is not discoverable from this instant on.
    pub discoverable_until: i64,
    /// The Share server confirmed the current publication lease.
    pub published: bool,
    /// The rights a device gets that pairs with this offer (plan 2026-10-31
    /// F5), as the worker holds them. `None`: the device connects without
    /// rights and waits for its confirmation (F1). Rooms never carry any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rights: Option<ConnectionRights>,
    /// The daemon published this offer after the previous one ended because
    /// too many attempts failed to prove its PIN (security line S9).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub renewed_after_failed_attempts: bool,
    /// The new PIN of a renewed offer, for the person to read out. It stays
    /// in the daemon's memory and reaches the surfaces over the local IPC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<DiscoveryPin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferLookup {
    Active(OwnDiscoveryOffer),
    Ended(DiscoveryOfferStopReason),
}

#[derive(Debug, Default)]
pub struct DiscoveryOfferBook {
    active: BTreeMap<String, OwnDiscoveryOffer>,
    /// Rights announced before their offer was seen (kept until it is).
    early_rights: BTreeMap<String, ConnectionRights>,
    ended: VecDeque<(String, DiscoveryOfferStopReason)>,
    /// Renewals of offers that ended after too many failed attempts.
    renewals: PendingRenewals,
}

impl DiscoveryOfferBook {
    pub fn observe(&mut self, event: &DiscoveryEvent) {
        self.observe_at(event, super::core::now_secs());
    }

    /// `observe` with the time of the event as an argument.
    pub fn observe_at(&mut self, event: &DiscoveryEvent, now: i64) {
        match event {
            DiscoveryEvent::OfferPrepared {
                offer_id,
                target,
                display_alias,
                discoverable_until,
            } => self.upsert(offer_id, target, display_alias, *discoverable_until, false),
            DiscoveryEvent::OfferPublished {
                offer_id,
                target,
                display_alias,
                discoverable_until,
            } => self.upsert(offer_id, target, display_alias, *discoverable_until, true),
            DiscoveryEvent::OfferRights { offer_id, rights } => {
                self.set_rights(offer_id, rights.clone())
            }
            DiscoveryEvent::OfferStopped { offer_id, reason } => self.end(offer_id, *reason, now),
            DiscoveryEvent::DiscoveryList { .. }
            | DiscoveryEvent::ExchangeStarted { .. }
            | DiscoveryEvent::ExchangeCompleted { .. }
            | DiscoveryEvent::ExchangeCancelled { .. }
            | DiscoveryEvent::ExchangeFailed { .. }
            | DiscoveryEvent::ExchangeUnconfirmed { .. } => {}
        }
    }

    /// Offers still discoverable at `now`, earliest end first.
    pub fn offers(&self, now: i64) -> Vec<OwnDiscoveryOffer> {
        let mut offers: Vec<_> = self
            .active
            .values()
            .filter(|offer| offer.discoverable_until > now)
            .cloned()
            .collect();
        offers.sort_by(|left, right| {
            left.discoverable_until
                .cmp(&right.discoverable_until)
                .then_with(|| left.offer_id.cmp(&right.offer_id))
        });
        offers
    }

    pub fn offer_for_target(
        &self,
        target: &DiscoveryPublishTarget,
        now: i64,
    ) -> Option<&OwnDiscoveryOffer> {
        self.active
            .values()
            .find(|offer| offer.target == *target && offer.discoverable_until > now)
    }

    pub fn lookup(&self, offer_id: &str) -> Option<OfferLookup> {
        if let Some(offer) = self.active.get(offer_id) {
            return Some(OfferLookup::Active(offer.clone()));
        }
        self.ended
            .iter()
            .rev()
            .find(|(ended, _)| ended == offer_id)
            .map(|(_, reason)| OfferLookup::Ended(*reason))
    }

    /// Ends every tracked offer because the worker that held them is gone and
    /// returns their ids so the caller can announce the stop.
    pub fn end_all(&mut self, reason: DiscoveryOfferStopReason) -> Vec<String> {
        let ids: Vec<String> = self.active.keys().cloned().collect();
        for offer_id in &ids {
            self.end(offer_id, reason, 0);
        }
        // Nothing renews without a worker that holds the offers.
        self.renewals.clear();
        ids
    }

    /// The renewals due at `now`, for the daemon loop to publish. Those whose
    /// target has an offer again (the person published meanwhile) or whose
    /// time ran out are dropped. A claimed renewal stays until `finish_renewal`.
    pub fn claim_due_renewals(&mut self, now: i64) -> Vec<PinRenewal> {
        let active = &self.active;
        self.renewals.claim_due(now, |target| {
            active
                .values()
                .any(|offer| offer.target == *target && offer.discoverable_until > now)
        })
    }

    /// A new random PIN for a renewal, from the generator of the other surfaces.
    pub fn fresh_renewal_pin() -> Result<DiscoveryPin, String> {
        pin_renewal::renewal_pin()
    }

    /// A renewal is wanted until the person stops the offer it replaces.
    pub fn renewal_pending(&self, replaces: &str) -> bool {
        self.renewals.is_pending(replaces)
    }

    /// The person stopped the offer `replaces` (or a surface did for them):
    /// no renewal follows. `true` when one was waiting.
    pub fn cancel_renewal(&mut self, replaces: &str) -> bool {
        self.renewals.cancel(replaces)
    }

    /// The claimed renewal is done; `false`: it was cancelled meanwhile.
    pub fn finish_renewal(&mut self, replaces: &str) -> bool {
        self.renewals.finish(replaces)
    }

    /// The claimed renewal failed to publish; it is tried again later.
    pub fn postpone_renewal(&mut self, replaces: &str, now: i64) {
        self.renewals.postpone(replaces, now);
    }

    /// Number of renewals waiting for their pause or their publication.
    pub fn pending_renewals(&self) -> usize {
        self.renewals.len()
    }

    /// The renewals in their pause, for every surface (E49); a stop of their
    /// `replaces` cancels them.
    pub fn waiting_renewals(&self) -> Vec<WaitingRenewal> {
        self.renewals.waiting()
    }

    /// Marks the offer the daemon published as a renewal and keeps its new PIN.
    pub fn mark_renewed(&mut self, offer_id: &str, pin: DiscoveryPin) -> bool {
        match self.active.get_mut(offer_id) {
            Some(offer) => {
                offer.renewed_after_failed_attempts = true;
                offer.pin = Some(pin);
                true
            }
            None => false,
        }
    }

    fn upsert(
        &mut self,
        offer_id: &str,
        target: &DiscoveryPublishTarget,
        display_alias: &str,
        discoverable_until: i64,
        published: bool,
    ) {
        // A lease renewal or lapse rewrites the offer; its rights and the
        // mark of a renewed offer stay.
        let (rights, renewed, pin) = match self.active.get(offer_id) {
            Some(known) => (
                known.rights.clone(),
                known.renewed_after_failed_attempts,
                known.pin.clone(),
            ),
            None => (self.early_rights.remove(offer_id), false, None),
        };
        self.active.insert(
            offer_id.to_string(),
            OwnDiscoveryOffer {
                offer_id: offer_id.to_string(),
                target: target.clone(),
                display_alias: display_alias.to_string(),
                discoverable_until,
                published,
                rights,
                renewed_after_failed_attempts: renewed,
                pin,
            },
        );
    }

    fn set_rights(&mut self, offer_id: &str, rights: Option<ConnectionRights>) {
        match self.active.get_mut(offer_id) {
            Some(offer) => offer.rights = rights,
            None => match rights {
                Some(rights) => {
                    self.early_rights.insert(offer_id.to_string(), rights);
                }
                None => {
                    self.early_rights.remove(offer_id);
                }
            },
        }
    }

    fn end(&mut self, offer_id: &str, reason: DiscoveryOfferStopReason, now: i64) {
        let ended = self.active.remove(offer_id);
        self.early_rights.remove(offer_id);
        match (reason, ended) {
            // Someone may be guessing the PIN: after a pause the daemon offers
            // the same target again with a new PIN. Every other reason, above
            // all the person's stop and a pairing, leaves no renewal.
            (DiscoveryOfferStopReason::TooManyFailedAttempts, Some(offer)) => {
                self.renewals.plan(
                    PinRenewal {
                        replaces: offer.offer_id,
                        target: offer.target,
                        display_alias: offer.display_alias,
                        rights: offer.rights,
                        discoverable_until: offer.discoverable_until,
                        due: now.saturating_add(PIN_RENEWAL_PAUSE_SECS),
                    },
                    now,
                );
            }
            // A repeated event for an offer that already ended keeps the plan.
            (DiscoveryOfferStopReason::TooManyFailedAttempts, None) => {}
            _ => {
                self.renewals.cancel(offer_id);
            }
        }
        self.ended.retain(|(ended, _)| ended != offer_id);
        self.ended.push_back((offer_id.to_string(), reason));
        while self.ended.len() > RECENTLY_ENDED_CAPACITY {
            self.ended.pop_front();
        }
    }
}

#[cfg(test)]
#[path = "discovery_offer_rights_snapshot_tests.rs"]
mod abgleich_task_tests;

#[cfg(test)]
#[path = "discovery_pin_renewal_task_tests.rs"]
mod pin_neu_task_tests;

#[cfg(test)]
mod tests {
    use super::{DiscoveryOfferBook, OfferLookup, RECENTLY_ENDED_CAPACITY};
    use crate::share::{DiscoveryEvent, DiscoveryOfferStopReason, DiscoveryPublishTarget};

    fn prepared(offer_id: &str, target: DiscoveryPublishTarget, until: i64) -> DiscoveryEvent {
        DiscoveryEvent::OfferPrepared {
            offer_id: offer_id.into(),
            target,
            display_alias: format!("alias-{offer_id}"),
            discoverable_until: until,
        }
    }

    fn published(offer_id: &str, target: DiscoveryPublishTarget, until: i64) -> DiscoveryEvent {
        DiscoveryEvent::OfferPublished {
            offer_id: offer_id.into(),
            target,
            display_alias: format!("alias-{offer_id}"),
            discoverable_until: until,
        }
    }

    fn room(id: &str) -> DiscoveryPublishTarget {
        DiscoveryPublishTarget::Room {
            room_profile_id: id.into(),
        }
    }

    #[test]
    fn cli_task_offer_book_tracks_prepared_published_and_stopped() {
        let cases: [(&str, fn()); 2] = [
            (
                "cli_task_offer_book_tracks_prepared_published_and_stopped",
                cli_task_offer_book_tracks_prepared_published_and_stopped_case,
            ),
            (
                "cli_task_offer_book_ends_all_and_bounds_recent_ends",
                cli_task_offer_book_ends_all_and_bounds_recent_ends,
            ),
        ];
        for (case, check) in cases {
            std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
        }
    }

    fn cli_task_offer_book_tracks_prepared_published_and_stopped_case() {
        let mut book = DiscoveryOfferBook::default();
        book.observe(&prepared("a", DiscoveryPublishTarget::Direct, 1_300));
        let offers = book.offers(1_000);
        assert_eq!(offers.len(), 1);
        assert!(!offers[0].published);
        assert_eq!(offers[0].display_alias, "alias-a");
        assert_eq!(offers[0].discoverable_until, 1_300);

        book.observe(&published("a", DiscoveryPublishTarget::Direct, 1_300));
        assert!(book.offers(1_000)[0].published);
        // A lapsed lease reports the offer as prepared again.
        book.observe(&prepared("a", DiscoveryPublishTarget::Direct, 1_300));
        assert!(!book.offers(1_000)[0].published);

        book.observe(&DiscoveryEvent::OfferStopped {
            offer_id: "a".into(),
            reason: DiscoveryOfferStopReason::Requested,
        });
        assert!(book.offers(1_000).is_empty());
        assert_eq!(
            book.lookup("a"),
            Some(OfferLookup::Ended(DiscoveryOfferStopReason::Requested))
        );
        assert_eq!(book.lookup("unknown"), None);
    }

    fn cli_task_offer_book_ends_all_and_bounds_recent_ends() {
        let mut book = DiscoveryOfferBook::default();
        book.observe(&prepared("a", DiscoveryPublishTarget::Direct, 9_000));
        book.observe(&published("b", room("r"), 9_000));
        let mut ended = book.end_all(DiscoveryOfferStopReason::WorkerStopped);
        ended.sort();
        assert_eq!(ended, ["a", "b"]);
        assert!(book.offers(0).is_empty());
        assert_eq!(
            book.lookup("b"),
            Some(OfferLookup::Ended(DiscoveryOfferStopReason::WorkerStopped))
        );

        for index in 0..RECENTLY_ENDED_CAPACITY + 4 {
            book.observe(&DiscoveryEvent::OfferStopped {
                offer_id: format!("x{index}"),
                reason: DiscoveryOfferStopReason::Expired,
            });
        }
        assert_eq!(book.lookup("a"), None);
        assert_eq!(
            book.lookup(&format!("x{}", RECENTLY_ENDED_CAPACITY + 3)),
            Some(OfferLookup::Ended(DiscoveryOfferStopReason::Expired))
        );
    }

    #[test]
    fn cli_task_offer_book_lists_only_unexpired_offers_sorted() {
        let mut book = DiscoveryOfferBook::default();
        book.observe(&prepared("late", room("r1"), 2_000));
        book.observe(&prepared("early", DiscoveryPublishTarget::Direct, 1_500));
        book.observe(&prepared("gone", room("r2"), 1_000));
        let ids: Vec<_> = book
            .offers(1_000)
            .into_iter()
            .map(|offer| offer.offer_id)
            .collect();
        assert_eq!(ids, ["early", "late"]);
        assert!(book
            .offer_for_target(&DiscoveryPublishTarget::Direct, 1_000)
            .is_some());
        assert!(book.offer_for_target(&room("r2"), 1_000).is_none());
        assert!(book.offer_for_target(&room("r1"), 1_000).is_some());
        assert!(book.offer_for_target(&room("r1"), 2_000).is_none());
        // Other discovery events never touch the own offers.
        book.observe(&DiscoveryEvent::DiscoveryList {
            advertisements: Vec::new(),
        });
        assert_eq!(book.offers(1_000).len(), 2);
    }
}
