//! F5 (plan 2026-10-31): the rights chosen with a PIN offer, kept by the
//! publishing worker beside the offer's crypto state and handed to the
//! relation store when a Direct device pairs with that offer.
use std::collections::HashMap;

use super::super::connection_rights::ConnectionRights;

#[derive(Default)]
pub(super) struct OfferRights {
    by_offer: HashMap<String, ConnectionRights>,
    by_exchange: HashMap<String, ConnectionRights>,
}

impl OfferRights {
    pub(super) fn set(&mut self, offer_id: &str, rights: Option<ConnectionRights>) {
        match rights {
            Some(rights) => self.by_offer.insert(offer_id.to_string(), rights),
            None => self.by_offer.remove(offer_id),
        };
    }

    /// A publisher exchange of `offer_id` started as `exchange_id`.
    pub(super) fn start(&mut self, offer_id: &str, exchange_id: &str) {
        if let Some(rights) = self.by_offer.get(offer_id).cloned() {
            self.by_exchange.insert(exchange_id.to_string(), rights);
        }
    }

    pub(super) fn of_exchange(&self, exchange_id: &str) -> Option<ConnectionRights> {
        self.by_exchange.get(exchange_id).cloned()
    }

    pub(super) fn end_exchange(&mut self, exchange_id: &str) {
        self.by_exchange.remove(exchange_id);
    }

    pub(super) fn remove_offer(&mut self, offer_id: &str) {
        self.by_offer.remove(offer_id);
    }
}
