//! Single use and attempt limit of this device's discovery offers (FC2):
//! once a connector proved the PIN no other exchange may start, and the
//! offer ends after that pairing or after `DISCOVERY_MAX_FAILED_PAIRINGS`
//! attempts that never proved it (OPAQUE lets a wrong PIN fail on the
//! connector, so the publisher sees cancelled or expired exchanges). Ends
//! are queued here and carried out by the next maintenance, which has the
//! signal connection at hand.

use std::collections::HashMap;

use super::discovery_pin::DISCOVERY_MAX_FAILED_PAIRINGS;
use super::discovery_signal_types::DiscoveryOfferStopReason;

#[derive(Default)]
pub(super) struct OfferGuards {
    offers: HashMap<String, OfferGuard>,
    due_stops: Vec<(String, DiscoveryOfferStopReason)>,
}

#[derive(Default)]
struct OfferGuard {
    failed: u8,
    paired: bool,
}

impl OfferGuards {
    /// A connector proved the PIN: no further exchange may start.
    pub(super) fn mark_paired(&mut self, offer_id: &str) {
        self.offers.entry(offer_id.to_string()).or_default().paired = true;
    }

    pub(super) fn is_paired(&self, offer_id: &str) -> bool {
        self.offers.get(offer_id).is_some_and(|guard| guard.paired)
    }

    /// The pairing finished (confirmed or not): the offer ends.
    pub(super) fn finish_paired(&mut self, offer_id: &str) {
        self.mark_paired(offer_id);
        self.queue_stop(offer_id, DiscoveryOfferStopReason::Paired);
    }

    /// An exchange ended after this side answered but before the PIN was
    /// proven. Returns true when the attempt ended the offer.
    pub(super) fn record_failure(&mut self, offer_id: &str) -> bool {
        let guard = self.offers.entry(offer_id.to_string()).or_default();
        if guard.paired {
            return false;
        }
        guard.failed = guard.failed.saturating_add(1);
        if guard.failed < DISCOVERY_MAX_FAILED_PAIRINGS {
            return false;
        }
        self.queue_stop(offer_id, DiscoveryOfferStopReason::TooManyFailedAttempts);
        true
    }

    fn queue_stop(&mut self, offer_id: &str, reason: DiscoveryOfferStopReason) {
        if !self.due_stops.iter().any(|(queued, _)| queued == offer_id) {
            self.due_stops.push((offer_id.to_string(), reason));
        }
    }

    pub(super) fn has_due_stops(&self) -> bool {
        !self.due_stops.is_empty()
    }

    pub(super) fn take_due_stops(&mut self) -> Vec<(String, DiscoveryOfferStopReason)> {
        std::mem::take(&mut self.due_stops)
    }

    /// The offer ended for any reason; its counters go with it.
    pub(super) fn forget(&mut self, offer_id: &str) {
        self.offers.remove(offer_id);
        self.due_stops.retain(|(queued, _)| queued != offer_id);
    }
}

#[cfg(test)]
mod review_task_tests {
    use super::OfferGuards;
    use crate::share::discovery_signal_types::DiscoveryOfferStopReason;
    use crate::share::DISCOVERY_MAX_FAILED_PAIRINGS;

    #[test]
    fn review_task_offer_ends_after_five_failed_attempts() {
        let mut guards = OfferGuards::default();
        for _ in 1..DISCOVERY_MAX_FAILED_PAIRINGS {
            assert!(!guards.record_failure("offer"));
        }
        assert!(!guards.has_due_stops());
        assert!(guards.record_failure("offer"));
        assert_eq!(
            guards.take_due_stops(),
            [(
                "offer".to_string(),
                DiscoveryOfferStopReason::TooManyFailedAttempts
            )]
        );
        guards.forget("offer");
        assert!(!guards.record_failure("offer"));
    }

    #[test]
    fn review_task_offer_is_single_use_after_the_first_pairing() {
        let mut guards = OfferGuards::default();
        assert!(!guards.is_paired("offer"));
        guards.mark_paired("offer");
        assert!(guards.is_paired("offer"));
        // Attempts after the PIN was proven do not count.
        for _ in 0..DISCOVERY_MAX_FAILED_PAIRINGS {
            assert!(!guards.record_failure("offer"));
        }
        guards.finish_paired("offer");
        guards.finish_paired("offer");
        assert_eq!(
            guards.take_due_stops(),
            [("offer".to_string(), DiscoveryOfferStopReason::Paired)]
        );
        assert!(!guards.has_due_stops());
    }
}
