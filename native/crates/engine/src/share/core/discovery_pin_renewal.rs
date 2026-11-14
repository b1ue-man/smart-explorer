//! Renewal of a PIN offer that ended because too many attempts failed to
//! prove its PIN (plan 2026-11-08 E49, security line S9). The daemon's offer
//! book plans one renewal per target when such an offer ends, the daemon loop
//! publishes it when it is due, and a stop by the person cancels it. Time is
//! always an argument, so every rule is testable without a clock.
// This file is a child of `discovery_offer_book`, so the Share modules are
// one level up.
use super::super::connection_rights::ConnectionRights;
use super::super::discovery_pin::suggest_discovery_pin;
use super::super::discovery_signal_types::{
    DiscoveryCommand, DiscoveryPin, DiscoveryPublishTarget,
};
use serde::{Deserialize, Serialize};

/// Pause between the end of the offer and its renewal; it limits guessing.
pub const PIN_RENEWAL_PAUSE_SECS: i64 = 30;
/// Wait before a renewal that could not be published is tried again.
const PIN_RENEWAL_RETRY_SECS: i64 = 30;

/// What the replaced offer was and when the renewal is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinRenewal {
    /// The ended offer this one replaces; a stop of that id cancels the renewal.
    pub replaces: String,
    pub target: DiscoveryPublishTarget,
    pub display_alias: String,
    /// The rights of the replaced offer (`None`: the paired device waits).
    pub rights: Option<ConnectionRights>,
    /// Unix seconds; the end of the replaced offer. Every renewal keeps it,
    /// so a chain of renewals never outlasts the time the person chose.
    pub discoverable_until: i64,
    /// Unix seconds; the renewal is published from this instant on.
    pub due: i64,
}

impl PinRenewal {
    /// Seconds the renewed offer lasts when published at `now`.
    pub fn remaining_secs(&self, now: i64) -> i64 {
        self.discoverable_until.saturating_sub(now)
    }

    /// The publish command of the renewal: the replaced offer's target, name
    /// and rights, `pin` as the new PIN and the remaining time as duration.
    /// `None` once no time remains. The PIN is a generated one, so the weak
    /// PIN opt-in is never given.
    pub fn publish_command(&self, pin: DiscoveryPin, now: i64) -> Option<DiscoveryCommand> {
        let duration_secs = u64::try_from(self.remaining_secs(now)).ok()?;
        (duration_secs > 0).then(|| DiscoveryCommand::Publish {
            target: self.target.clone(),
            display_alias: self.display_alias.clone(),
            pin,
            duration_secs,
            allow_weak_pin: false,
            rights: self.rights.clone(),
        })
    }
}

/// A renewal in its pause, as every surface shows it (E49): the offer it
/// replaces, the target and name of the renewed offer, and when the new
/// offer with a new PIN goes out. Stopping `replaces` cancels it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WaitingRenewal {
    pub replaces: String,
    pub target: DiscoveryPublishTarget,
    pub display_alias: String,
    /// Unix seconds; the renewed offer is published from this instant on.
    pub due: i64,
}

/// A fresh random PIN from the generator the other surfaces use.
pub fn renewal_pin() -> Result<DiscoveryPin, String> {
    suggest_discovery_pin().map(DiscoveryPin::new)
}

#[derive(Debug)]
struct Entry {
    renewal: PinRenewal,
    /// Handed to the daemon loop and not finished yet.
    claimed: bool,
}

/// The renewals waiting for their pause, at most one per target.
#[derive(Debug, Default)]
pub(super) struct PendingRenewals {
    entries: Vec<Entry>,
}

impl PendingRenewals {
    /// Plans the renewal of an offer that ended at `now`; nothing when no
    /// time remains. A renewal planned earlier for the same target is replaced.
    pub(super) fn plan(&mut self, renewal: PinRenewal, now: i64) {
        if renewal.remaining_secs(now) <= 0 {
            return;
        }
        self.entries
            .retain(|entry| entry.renewal.target != renewal.target);
        self.entries.push(Entry {
            renewal,
            claimed: false,
        });
    }

    /// Drops the renewal of an offer the person stopped; `true` when one waited.
    pub(super) fn cancel(&mut self, replaces: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.renewal.replaces != replaces);
        self.entries.len() != before
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn is_pending(&self, replaces: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.renewal.replaces == replaces)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The renewals still in their pause or waiting for a retry; a claimed
    /// one is being published.
    pub(super) fn waiting(&self) -> Vec<WaitingRenewal> {
        self.entries
            .iter()
            .filter(|entry| !entry.claimed)
            .map(|entry| WaitingRenewal {
                replaces: entry.renewal.replaces.clone(),
                target: entry.renewal.target.clone(),
                display_alias: entry.renewal.display_alias.clone(),
                due: entry.renewal.due,
            })
            .collect()
    }

    /// Hands out the renewals due at `now` for publishing. Renewals without
    /// remaining time and those whose target already has an offer (`blocked`)
    /// are dropped.
    pub(super) fn claim_due(
        &mut self,
        now: i64,
        blocked: impl Fn(&DiscoveryPublishTarget) -> bool,
    ) -> Vec<PinRenewal> {
        self.entries.retain(|entry| {
            let hopeless = entry.renewal.remaining_secs(now) <= 0 || blocked(&entry.renewal.target);
            entry.claimed || entry.renewal.due > now || !hopeless
        });
        let mut due = Vec::new();
        for entry in &mut self.entries {
            if !entry.claimed && entry.renewal.due <= now {
                entry.claimed = true;
                due.push(entry.renewal.clone());
            }
        }
        due
    }

    /// The renewal is published (or hopeless): `true` when it was still
    /// wanted, `false` when the person cancelled it meanwhile.
    pub(super) fn finish(&mut self, replaces: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.renewal.replaces != replaces);
        self.entries.len() != before
    }

    /// The renewal could not be published; it is tried again later.
    pub(super) fn postpone(&mut self, replaces: &str, now: i64) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.renewal.replaces == replaces)
        {
            entry.claimed = false;
            entry.renewal.due = now.saturating_add(PIN_RENEWAL_RETRY_SECS);
        }
    }
}
