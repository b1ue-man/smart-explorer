//! PIN drafts and unconfirmed pairings of the discovery client state (FC2):
//! strength checks and suggestions for PIN fields, and pairings that were
//! installed without the other side's confirmation ("gekoppelt – Bestätigung
//! fehlt") together with what "Widerrufen" removes.

use zeroize::Zeroize;

use super::DiscoveryUiState;
use crate::share::{DiscoveryPinStrength, DiscoveryRelationOutcome};

#[derive(Default)]
pub struct DiscoveryPinDraft(String);

impl std::fmt::Debug for DiscoveryPinDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DiscoveryPinDraft([REDACTED])")
    }
}

impl Drop for DiscoveryPinDraft {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl DiscoveryPinDraft {
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.0
    }

    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn strength(&self) -> DiscoveryPinStrength {
        crate::share::discovery_pin_strength(self.0.as_bytes())
    }

    /// Short or trivial: publishing it needs "Unsichere PIN erlauben".
    pub fn trivially_guessable(&self) -> bool {
        !self.strength().is_acceptable()
    }

    /// Replaces the draft with a fresh random six-digit PIN.
    pub fn suggest(&mut self) -> Result<(), String> {
        let suggestion = crate::share::suggest_discovery_pin()?;
        self.0.zeroize();
        self.0 = suggestion;
        Ok(())
    }
}

/// Status and exchange label of a pairing that lacks confirmation.
pub fn unconfirmed_label(outcome: &DiscoveryRelationOutcome) -> String {
    match outcome {
        DiscoveryRelationOutcome::DirectInstalled { display_name, .. } => {
            format!("Gekoppelt mit {display_name} – Bestätigung fehlt; bei Zweifel widerrufen")
        }
        DiscoveryRelationOutcome::RoomInstalled { display_name, .. } => {
            format!("Raum {display_name} hinzugefügt – Bestätigung fehlt; bei Zweifel widerrufen")
        }
        DiscoveryRelationOutcome::RoomShared { display_name, .. } => format!(
            "Raum {display_name} übergeben – Bestätigung fehlt; das Gerät kennt den Raum womöglich"
        ),
    }
}

/// What "Widerrufen" can remove: the installed contact or room. Handed-out
/// Room material cannot be taken back.
pub fn revocable(outcome: &DiscoveryRelationOutcome) -> bool {
    !matches!(outcome, DiscoveryRelationOutcome::RoomShared { .. })
}

impl DiscoveryUiState {
    /// The relation stays installed; it is shown with "Widerrufen" instead
    /// of as a failure (S24).
    pub fn exchange_unconfirmed(
        &mut self,
        exchange_id: String,
        discovery_id: Option<String>,
        outcome: DiscoveryRelationOutcome,
    ) {
        let label = unconfirmed_label(&outcome);
        let known_discovery = discovery_id.or_else(|| {
            self.exchanges
                .get(&exchange_id)
                .map(|record| record.discovery_id.clone())
        });
        if let Some(discovery_id) = known_discovery {
            self.starting_discoveries.remove(&discovery_id);
            super::super::discovery_retention::replace_exchange_record(
                self,
                exchange_id.clone(),
                super::DiscoveryExchangeRecord {
                    discovery_id,
                    state: super::DiscoveryExchangeState::Complete(label.clone()),
                },
            );
        }
        self.unconfirmed.insert(exchange_id, outcome);
        self.status = Some(label);
    }

    /// Takes the pairing "Widerrufen" acts on.
    pub fn take_unconfirmed(&mut self, exchange_id: &str) -> Option<DiscoveryRelationOutcome> {
        self.unconfirmed.remove(exchange_id)
    }
}
