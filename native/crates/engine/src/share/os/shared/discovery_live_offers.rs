//! The GUI state's view of the offers the daemon's Share worker really runs
//! (plan 2026-10-31 F5): the daemon's offer list is authoritative for which
//! offers exist and for the rights a device gets that pairs with them, so a
//! restarted interface shows the same as the one that published the offer.

use super::{ActiveDiscoveryOffer, DiscoveryOfferPhase, DiscoveryUiState, WORKER_STOPPED_STATUS};
use crate::share::connection_rights::ConnectionRights;

impl DiscoveryUiState {
    /// Folds the daemon's own-offer list in: drops offers the worker no
    /// longer runs (for example after a handover to a new version), takes up
    /// offers the interface does not know yet (it was restarted while the
    /// offer lives) and copies the rights of every offer.
    pub fn retain_live_offers(&mut self, live: &[crate::share::OwnDiscoveryOffer]) {
        let before = self.active_offers.len();
        self.active_offers
            .retain(|offer| live.iter().any(|live| live.offer_id == offer.offer_id));
        if self.active_offers.len() < before {
            self.status = Some(WORKER_STOPPED_STATUS.to_string());
        }
        for offer in live {
            if !self
                .active_offers
                .iter()
                .any(|known| known.offer_id == offer.offer_id)
            {
                self.adopt_live_offer(offer);
            }
            self.set_offer_rights(&offer.offer_id, offer.rights.clone());
        }
    }

    /// The rights of one offer, as the worker announced them.
    pub fn set_offer_rights(&mut self, offer_id: &str, rights: Option<ConnectionRights>) {
        if let Some(offer) = self
            .active_offers
            .iter_mut()
            .find(|offer| offer.offer_id == offer_id)
        {
            offer.rights = rights;
        }
    }

    fn adopt_live_offer(&mut self, live: &crate::share::OwnDiscoveryOffer) {
        let target = super::super::discovery_events::map_publish_target(
            live.target.clone(),
            live.display_alias.clone(),
        );
        let phase = if live.published {
            DiscoveryOfferPhase::Published
        } else {
            DiscoveryOfferPhase::Prepared
        };
        // `offer_updated` also clears the pending publish flag of the
        // target; its status text is for fresh offers only.
        let status = self.status.take();
        self.offer_updated(
            live.offer_id.clone(),
            target,
            live.discoverable_until,
            phase,
        );
        self.status = status;
        if let Some(adopted) = self
            .active_offers
            .iter_mut()
            .find(|offer| offer.offer_id == live.offer_id)
        {
            adopted.alias = live.display_alias.clone();
        }
    }
}

impl ActiveDiscoveryOffer {
    /// The sentence naming what a device pairing with this offer gets.
    pub fn rights_sentence(&self) -> String {
        match &self.rights {
            Some(rights) if !rights.grants_anything() => {
                "Das Gerät, das mit dieser PIN verbindet, wird ohne Rechte verbunden.".to_string()
            }
            Some(rights) => format!(
                "Das Gerät, das mit dieser PIN verbindet, erhält: {}.",
                rights.summary()
            ),
            None => "Das Gerät, das mit dieser PIN verbindet, wartet ohne Rechte; du legst sie \
                     danach fest."
                .to_string(),
        }
    }
}

#[cfg(test)]
#[path = "discovery_live_offers_tests.rs"]
mod abgleich_task_tests;
