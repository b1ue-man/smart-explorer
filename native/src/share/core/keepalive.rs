use std::time::Duration;

use iroh::endpoint::{QuicTransportConfig, VarInt};

pub(super) const IROH_CONNECTION_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const IROH_PATH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const IROH_CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const SIGNAL_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
pub(super) const SIGNAL_PONG_TIMEOUT: Duration = Duration::from_secs(40);
pub(super) const SIGNAL_PRESENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
pub(super) const SIGNAL_TRACKED_OUTBOX_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SignalMaintenancePolicy {
    heartbeat_interval: Duration,
    presence_refresh_interval: Duration,
    tracked_outbox_interval: Duration,
    pong_timeout: Duration,
}

pub(super) const SIGNAL_MAINTENANCE_POLICY: SignalMaintenancePolicy = SignalMaintenancePolicy {
    heartbeat_interval: SIGNAL_HEARTBEAT_INTERVAL,
    presence_refresh_interval: SIGNAL_PRESENCE_REFRESH_INTERVAL,
    tracked_outbox_interval: SIGNAL_TRACKED_OUTBOX_INTERVAL,
    pong_timeout: SIGNAL_PONG_TIMEOUT,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SignalMaintenanceDue {
    pub(super) heartbeat: bool,
    pub(super) presence_refresh: bool,
    pub(super) tracked_outbox: bool,
}

impl SignalMaintenancePolicy {
    pub(super) fn due(
        self,
        heartbeat_elapsed: Duration,
        presence_elapsed: Duration,
        tracked_elapsed: Duration,
        tracked_direct: bool,
    ) -> SignalMaintenanceDue {
        SignalMaintenanceDue {
            heartbeat: heartbeat_elapsed >= self.heartbeat_interval,
            presence_refresh: presence_elapsed >= self.presence_refresh_interval,
            tracked_outbox: tracked_direct && tracked_elapsed >= self.tracked_outbox_interval,
        }
    }

    pub(super) fn pong_expired(self, outstanding_for: Option<Duration>) -> bool {
        outstanding_for.is_some_and(|elapsed| elapsed >= self.pong_timeout)
    }
}

pub(super) fn iroh_transport_config() -> QuicTransportConfig {
    QuicTransportConfig::builder()
        // Live idle peers remain open because the transport sends a keepalive
        // every five seconds. A crashed peer cannot answer those probes and is
        // therefore reported to an active Exec client within a bounded time.
        .max_idle_timeout(Some(
            VarInt::from_u32(IROH_CONNECTION_IDLE_TIMEOUT.as_millis() as u32).into(),
        ))
        .keep_alive_interval(IROH_CONNECTION_KEEPALIVE_INTERVAL)
        .default_path_keep_alive_interval(IROH_PATH_KEEPALIVE_INTERVAL)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_maintenance_fires_at_thresholds() {
        let before = SIGNAL_MAINTENANCE_POLICY.due(
            SIGNAL_HEARTBEAT_INTERVAL - Duration::from_millis(1),
            SIGNAL_PRESENCE_REFRESH_INTERVAL - Duration::from_millis(1),
            SIGNAL_TRACKED_OUTBOX_INTERVAL - Duration::from_millis(1),
            true,
        );
        assert_eq!(before, SignalMaintenanceDue::default());

        let due = SIGNAL_MAINTENANCE_POLICY.due(
            SIGNAL_HEARTBEAT_INTERVAL,
            SIGNAL_PRESENCE_REFRESH_INTERVAL,
            SIGNAL_TRACKED_OUTBOX_INTERVAL,
            true,
        );
        assert_eq!(
            due,
            SignalMaintenanceDue {
                heartbeat: true,
                presence_refresh: true,
                tracked_outbox: true,
            }
        );

        let untracked = SIGNAL_MAINTENANCE_POLICY.due(
            Duration::ZERO,
            Duration::ZERO,
            SIGNAL_TRACKED_OUTBOX_INTERVAL,
            false,
        );
        assert!(!untracked.tracked_outbox);

        assert!(!SIGNAL_MAINTENANCE_POLICY.pong_expired(None));
        assert!(!SIGNAL_MAINTENANCE_POLICY
            .pong_expired(Some(SIGNAL_PONG_TIMEOUT - Duration::from_millis(1))));
        assert!(SIGNAL_MAINTENANCE_POLICY.pong_expired(Some(SIGNAL_PONG_TIMEOUT)));
    }

    #[test]
    fn iroh_transport_keepalives_are_explicit_and_nonzero() {
        assert_eq!(IROH_CONNECTION_KEEPALIVE_INTERVAL, Duration::from_secs(5));
        assert_eq!(IROH_PATH_KEEPALIVE_INTERVAL, Duration::from_secs(5));
        assert_eq!(IROH_CONNECTION_IDLE_TIMEOUT, Duration::from_secs(20));
        assert!(!IROH_CONNECTION_KEEPALIVE_INTERVAL.is_zero());
        assert!(!IROH_PATH_KEEPALIVE_INTERVAL.is_zero());

        let _config = iroh_transport_config();
    }
}
