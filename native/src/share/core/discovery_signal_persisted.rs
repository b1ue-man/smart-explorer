use super::configuration_runtime::RuntimeConfiguration;
use super::discovery_signal_commands::DiscoverySignalRuntime;
use super::discovery_signal_exchange::{canonical_payload_text_len, send_pairing_packet};
use super::discovery_signal_port::PersistedDiscoveryPacket;
use super::signal_connection::SignalConnection;
use super::types::ShareEvent;

impl DiscoverySignalRuntime {
    pub(super) fn apply_persisted_and_send(
        &mut self,
        exchange_id: &str,
        persisted: PersistedDiscoveryPacket,
        signal: &mut SignalConnection,
        events: &crossbeam_channel::Sender<ShareEvent>,
        configuration: &mut RuntimeConfiguration<'_>,
        tracked_direct: bool,
    ) -> Result<(), String> {
        let (commit, packet) = persisted.into_parts();
        self.state
            .exchanges
            .get_mut(exchange_id)
            .ok_or("Discovery-Austausch verschwand vor dem Commit-Senden")
            .and_then(|exchange| {
                exchange.accept_port_packet(packet.kind)?;
                exchange.record_payload(canonical_payload_text_len(packet.payload.len()))
            })?;
        let teardown = super::signal_commands::plan_current_subscription_teardown(
            configuration.auth,
            &commit.profiles().direct_contacts,
            &commit.profiles().rooms,
        )
        .map_err(|error| error.to_string())?;

        // Persistence is already authoritative. Notify the daemon even if a
        // later live transition or send fails; its queued Configure cannot run
        // re-entrantly on this worker while this exchange is still on-stack.
        let _ = events.send(ShareEvent::RuntimeProfilesCommitted);
        configuration
            .apply_profiles(commit.into_parts().0)
            .map_err(|error| error.to_string())?;
        send_pairing_packet(signal, exchange_id, packet.kind, &packet.payload)
            .map_err(|error| error.to_string())?;
        super::signal_commands::send_subscription_teardown(signal, teardown)
            .map_err(|error| error.to_string())?;
        super::signal_worker::publish_all(
            signal,
            configuration.auth,
            configuration.iroh,
            configuration.direct_requests_sent,
            tracked_direct,
        )
        .map_err(|error| error.to_string())
    }
}
