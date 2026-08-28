use std::time::{Duration, Instant};

use super::core::now_secs;
use super::discovery_signal_commands::{
    offer_state_event, send_discovery_event, DiscoverySignalRuntime,
};
use super::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryEvent, DiscoveryOfferStopReason,
};
use super::discovery_signal_validation::validate_advertisement;
use super::signal_connection::SignalConnection;
use super::types::ShareEvent;

impl DiscoverySignalRuntime {
    pub(super) fn handle_published(
        &mut self,
        advertisement: DiscoveryAdvertisement,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
    ) -> Result<(), String> {
        validate_advertisement(&advertisement)?;
        if !self.state.offers.contains_key(&advertisement.offer_id) {
            if self.state.is_closed_offer(&advertisement.offer_id) {
                return Ok(());
            }
            return Err("Server bestaetigte ein unbekanntes Discovery-Offer".into());
        }
        if self.state.offers.values().any(|offer| {
            offer.offer_id != advertisement.offer_id
                && offer.discovery_id.as_deref() == Some(&advertisement.discovery_id)
        }) {
            return Err("Server band dieselbe Discovery-ID an mehrere Offers".into());
        }
        let publication_event = {
            let now = Instant::now();
            let wall_remaining = advertisement.expires_at.saturating_sub(now_secs());
            if wall_remaining <= 0 {
                return Err("Discovery-Publish-Bestaetigung ist bereits abgelaufen".into());
            }
            let server_until = now
                .checked_add(Duration::from_secs(wall_remaining as u64))
                .ok_or("Discovery-Server-Lease ist lokal nicht darstellbar")?;
            let offer = self
                .state
                .offers
                .get_mut(&advertisement.offer_id)
                .ok_or("Discovery-Offer verschwand waehrend der Bestaetigung")?;
            if !offer.accepts_advertisement(&advertisement) {
                return Err("Server veraenderte die gebundenen Discovery-Metadaten".into());
            }
            let sent_at = offer
                .last_publish_sent_at
                .take()
                .ok_or("Server bestaetigte kein ausstehendes Discovery-Publish")?;
            let local_until = sent_at
                .checked_add(Duration::from_secs(u64::from(offer.last_publish_lease_secs)))
                .ok_or("Discovery-Lease ist lokal nicht darstellbar")?;
            let acknowledged_until = std::cmp::min(
                offer.deadline,
                std::cmp::min(local_until, server_until),
            );
            if acknowledged_until <= now {
                return Err("Discovery-Publish-Bestaetigung traf nach Lease-Ablauf ein".into());
            }
            let was_published = offer.published_until.is_some_and(|until| until > now);
            offer.published_until = Some(acknowledged_until);
            offer.discovery_id = Some(advertisement.discovery_id.clone());
            (!was_published).then(|| offer_state_event(offer, true))
        };
        if let Some(event) = publication_event {
            send_discovery_event(events, event);
        }
        let pending = self
            .state
            .pending_publisher_starts
            .remove(&advertisement.discovery_id)
            .unwrap_or_default();
        for start in pending {
            if start.offer_id != advertisement.offer_id {
                self.fail_exchange(
                    signal,
                    &start.exchange_id,
                    Some(start.discovery_id),
                    "Discovery-Start passt nicht zur Publish-Bestaetigung",
                    events,
                )?;
                return Err("Server band einen vorzeitigen Start an das falsche Offer".into());
            }
            if let Err(error) = self.start_publisher_exchange(
                start.exchange_id.clone(),
                start.discovery_id.clone(),
                advertisement.offer_id.clone(),
                start.payload,
                start.deadline,
                signal,
                events,
            ) {
                let transport = error.transport;
                let target_unavailable = error.target_unavailable;
                self.fail_exchange(
                    signal,
                    &error.exchange_id,
                    Some(start.discovery_id),
                    if target_unavailable {
                        "Discovery-Ziel ist nicht mehr verfuegbar"
                    } else {
                        "Discovery-Publisher konnte den Austausch nicht starten"
                    },
                    events,
                )?;
                if target_unavailable {
                    self.stop_offer_connected(
                        signal,
                        &advertisement.offer_id,
                        DiscoveryOfferStopReason::TargetUnavailable,
                        Some("Discovery-Ziel ist nicht mehr verfuegbar"),
                        events,
                    )
                    .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                if transport {
                    return Err("Discovery-Nachricht konnte nicht gesendet werden".into());
                }
            }
        }
        Ok(())
    }
}
