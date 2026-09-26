//! The daemon's durable view of this device's own discovery offers. The Share
//! worker reports every offer transition as a `DiscoveryEvent`; folding them
//! here keeps the state readable by every client, independent of who drains
//! the shared UI event buffer.
use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::discovery_signal_types::{
    DiscoveryEvent, DiscoveryOfferStopReason, DiscoveryPublishTarget,
};

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferLookup {
    Active(OwnDiscoveryOffer),
    Ended(DiscoveryOfferStopReason),
}

#[derive(Debug, Default)]
pub struct DiscoveryOfferBook {
    active: BTreeMap<String, OwnDiscoveryOffer>,
    ended: VecDeque<(String, DiscoveryOfferStopReason)>,
}

impl DiscoveryOfferBook {
    pub fn observe(&mut self, event: &DiscoveryEvent) {
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
            DiscoveryEvent::OfferStopped { offer_id, reason } => self.end(offer_id, *reason),
            DiscoveryEvent::DiscoveryList { .. }
            | DiscoveryEvent::ExchangeStarted { .. }
            | DiscoveryEvent::ExchangeCompleted { .. }
            | DiscoveryEvent::ExchangeCancelled { .. }
            | DiscoveryEvent::ExchangeFailed { .. } => {}
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
            self.end(offer_id, reason);
        }
        ids
    }

    fn upsert(
        &mut self,
        offer_id: &str,
        target: &DiscoveryPublishTarget,
        display_alias: &str,
        discoverable_until: i64,
        published: bool,
    ) {
        self.active.insert(
            offer_id.to_string(),
            OwnDiscoveryOffer {
                offer_id: offer_id.to_string(),
                target: target.clone(),
                display_alias: display_alias.to_string(),
                discoverable_until,
                published,
            },
        );
    }

    fn end(&mut self, offer_id: &str, reason: DiscoveryOfferStopReason) {
        self.active.remove(offer_id);
        self.ended.retain(|(ended, _)| ended != offer_id);
        self.ended.push_back((offer_id.to_string(), reason));
        while self.ended.len() > RECENTLY_ENDED_CAPACITY {
            self.ended.pop_front();
        }
    }
}

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

    #[test]
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
}
