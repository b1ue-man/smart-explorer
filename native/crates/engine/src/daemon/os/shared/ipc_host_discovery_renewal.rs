//! Publishes the renewal of a PIN offer that ended after too many failed
//! attempts (plan 2026-11-08 E49, security line S9). The offer book plans the
//! renewal and names it when its pause is over; this loop step publishes it
//! over the one path every client uses (`send_command`: one running offer per
//! target, publish lock), with a new PIN and the remaining time. The PIN is
//! never written to a log.
use crate::share::{DiscoveryCommand, DiscoveryOfferBook, ShareCmd};

use super::super::ipc_host::ShareHost;
use super::super::ipc_protocol::ShareCommandReply;
use super::super::state::log;

impl ShareHost {
    /// One step of the daemon loop: publishes every renewal whose pause is over.
    /// The embedded (Android) and the desktop daemon run the same loop.
    pub(in crate::daemon) fn renew_due_discovery_offers(&self) {
        let now = crate::share::core_now_secs();
        let due = match self.state.lock() {
            Ok(mut state) => state.discovery_offers.claim_due_renewals(now),
            Err(_) => return,
        };
        for renewal in due {
            let replaces = renewal.replaces.clone();
            // The person may have stopped the replaced offer since the claim.
            if !self.renewal_wanted(&replaces) {
                continue;
            }
            let pin = match DiscoveryOfferBook::fresh_renewal_pin() {
                Ok(pin) => pin,
                Err(error) => {
                    log(&format!(
                        "discovery renewal: no PIN ({error}); trying again"
                    ));
                    self.postpone_renewal(&replaces, now);
                    continue;
                }
            };
            match renewal.publish_command(pin.clone(), now) {
                Some(command) => self.publish_renewal(&replaces, now, command, pin),
                // No time remains: the offer would end at once.
                None => self.finish_renewal(&replaces),
            }
        }
    }

    fn publish_renewal(
        &self,
        replaces: &str,
        now: i64,
        command: DiscoveryCommand,
        pin: crate::share::DiscoveryPin,
    ) {
        match self.send_command(ShareCmd::Discovery(command)) {
            Ok(ShareCommandReply::DiscoveryOffer { offer }) => {
                self.accept_renewed_offer(replaces, &offer.offer_id, pin)
            }
            // Ended right away (for example the server lost discovery): its
            // reason is in the book and is not a failed attempt, so no renewal.
            Ok(_) => self.finish_renewal(replaces),
            Err(error) => {
                log(&format!("discovery renewal: not published ({error})"));
                self.postpone_renewal(replaces, now);
            }
        }
    }

    /// Marks the published offer as a renewal; when the person stopped the
    /// replaced offer while it was published, the new offer is withdrawn.
    fn accept_renewed_offer(
        &self,
        replaces: &str,
        offer_id: &str,
        pin: crate::share::DiscoveryPin,
    ) {
        let wanted = match self.state.lock() {
            Ok(mut state) => {
                let wanted = state.discovery_offers.finish_renewal(replaces);
                if wanted {
                    state.discovery_offers.mark_renewed(offer_id, pin);
                }
                wanted
            }
            Err(_) => return,
        };
        if !wanted {
            let stop = DiscoveryCommand::StopPublishing {
                offer_id: offer_id.to_string(),
            };
            if let Err(error) = self.send_command(ShareCmd::Discovery(stop)) {
                log(&format!("discovery renewal: withdrawing failed ({error})"));
            }
        }
    }

    fn renewal_wanted(&self, replaces: &str) -> bool {
        self.state
            .lock()
            .map(|state| state.discovery_offers.renewal_pending(replaces))
            .unwrap_or(false)
    }

    fn finish_renewal(&self, replaces: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.discovery_offers.finish_renewal(replaces);
        }
    }

    fn postpone_renewal(&self, replaces: &str, now: i64) {
        if let Ok(mut state) = self.state.lock() {
            state.discovery_offers.postpone_renewal(replaces, now);
        }
    }
}
