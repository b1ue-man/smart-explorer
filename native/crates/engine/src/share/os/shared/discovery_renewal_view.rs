//! What every surface shows for an own PIN offer the daemon renewed after too
//! many failed attempts (plan 2026-11-08 E49, security line S9): the hint
//! and the new PIN. The hint text and its number come from one place, so the
//! command line, the desktop, Android and the MCP tool name the same count.

use super::{ActiveDiscoveryOffer, DiscoveryPublishTarget, DiscoveryUiState};
use crate::share::{
    DiscoveryPin, OwnDiscoveryOffer, WaitingRenewal, DISCOVERY_MAX_FAILED_PAIRINGS,
};

const HINT_TAIL: &str = " falsche PIN-Versuche – vermutlich Rateversuche; neue PIN";

/// The hint text: the attempt count of `DISCOVERY_MAX_FAILED_PAIRINGS` and the
/// fixed wording (`format!` is no constant expression, so this is a function).
pub fn pin_renewal_hint() -> String {
    format!("{DISCOVERY_MAX_FAILED_PAIRINGS}{HINT_TAIL}")
}

/// Hint and new PIN of a renewed offer. The PIN keeps the redacted `Debug`
/// output of `DiscoveryPin`; `pin_text` is for display only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenewalNotice {
    hint: String,
    pin: Option<DiscoveryPin>,
}

impl RenewalNotice {
    /// The notice of an offer, `None` unless the daemon renewed it.
    pub fn of(renewed_after_failed_attempts: bool, pin: Option<&DiscoveryPin>) -> Option<Self> {
        renewed_after_failed_attempts.then(|| Self {
            hint: pin_renewal_hint(),
            pin: pin.cloned(),
        })
    }

    pub fn hint(&self) -> &str {
        &self.hint
    }

    /// The new PIN to read out; `None` when the daemon did not report it.
    pub fn pin_text(&self) -> Option<String> {
        let pin = self.pin.as_ref()?;
        Some(String::from_utf8_lossy(pin.as_bytes()).into_owned())
    }
}

impl OwnDiscoveryOffer {
    pub fn renewal_notice(&self) -> Option<RenewalNotice> {
        RenewalNotice::of(self.renewed_after_failed_attempts, self.pin.as_ref())
    }
}

impl ActiveDiscoveryOffer {
    pub fn renewal_notice(&self) -> Option<RenewalNotice> {
        RenewalNotice::of(self.renewed_after_failed_attempts, self.pin.as_ref())
    }
}

impl DiscoveryUiState {
    /// Copies the renewal facts of the daemon's offer list into the offer the
    /// interface shows (the rights travel the same way).
    pub fn adopt_live_renewal(&mut self, live: &OwnDiscoveryOffer) {
        self.set_offer_renewal(
            &live.offer_id,
            live.renewed_after_failed_attempts,
            live.pin.clone(),
        );
    }

    pub fn set_offer_renewal(&mut self, offer_id: &str, renewed: bool, pin: Option<DiscoveryPin>) {
        if let Some(offer) = self
            .active_offers
            .iter_mut()
            .find(|offer| offer.offer_id == offer_id)
        {
            offer.renewed_after_failed_attempts = renewed;
            offer.pin = pin;
        }
    }
}

/// What a surface shows for a renewal in its pause (E49); its stop is
/// „Nicht erneuern“ (`DiscoveryUiAction::Stop` of `replaces`).
pub fn waiting_renewal_text(renewal: &WaitingRenewal, now: i64) -> String {
    format!(
        "Neues Angebot mit neuer PIN in {} s",
        renewal.due.saturating_sub(now).max(0)
    )
}

impl DiscoveryUiState {
    /// Takes the renewals in their pause from the worker's snapshot. A stop
    /// of a renewal ends without an offer event: it is done once the
    /// renewal left the snapshot.
    pub fn set_waiting_renewals(&mut self, renewals: &[WaitingRenewal]) {
        for old in &self.waiting_renewals {
            if !renewals.iter().any(|new| new.replaces == old.replaces) {
                self.pending_stops.remove(&old.replaces);
            }
        }
        self.waiting_renewals = renewals.to_vec();
    }

    /// The renewal in its pause for the target the interface shows.
    pub fn waiting_renewal_for(&self, target: &DiscoveryPublishTarget) -> Option<&WaitingRenewal> {
        self.waiting_renewals
            .iter()
            .find(|renewal| match (&renewal.target, target) {
                (crate::share::DiscoveryPublishTarget::Direct, DiscoveryPublishTarget::Direct) => {
                    true
                }
                (
                    crate::share::DiscoveryPublishTarget::Room { room_profile_id },
                    DiscoveryPublishTarget::Room { room_id, .. },
                ) => room_profile_id == room_id,
                _ => false,
            })
    }
}
