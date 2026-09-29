use std::time::Duration;

use iroh::endpoint::{QuicTransportConfig, VarInt};

pub(super) const IROH_CONNECTION_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const IROH_PATH_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const IROH_CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const IROH_MAX_CONCURRENT_BIDI_STREAMS: u32 = 64;
pub(super) const IROH_MAX_CONCURRENT_UNI_STREAMS: u32 = 0;
/// Streams of one connection kept free for browsing, mounts and status
/// queries (research §2: four of the 64), so a full set of transfers never
/// stalls navigation on the same connection.
pub(super) const CONTROL_STREAM_RESERVE: u32 = 4;
/// Transfers one connection runs at once: its streams minus the reserve.
pub(super) const TRANSFER_STREAMS_PER_CONNECTION: u32 =
    IROH_MAX_CONCURRENT_BIDI_STREAMS - CONTROL_STREAM_RESERVE;
/// Bytes one stream may have in flight (research §3.1, ref A.4): 1 Gbit/s at
/// a 100 ms round trip is 12.5 MB, so 16 MiB lets one file fill such a link;
/// noq's default (1.25 MB, sized for 100 Mbit/s at 100 ms) caps it there.
const STREAM_RECEIVE_WINDOW: u32 = 16 * 1024 * 1024;
/// Bytes all streams of one connection may have in flight: four full-speed
/// streams at once (research §3.1: 64 MiB).
const CONNECTION_RECEIVE_WINDOW: u32 = 4 * STREAM_RECEIVE_WINDOW;
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

/// QUIC flow-control windows of this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransportWindows {
    pub(super) stream: u32,
    pub(super) connection: u32,
    pub(super) send: u64,
}

/// Windows for a process whose transfer memory budget is `budget` bytes.
/// They are credit, not allocations: receive memory is held only while the
/// application reads slower than the network delivers, send memory only
/// while sent data awaits acknowledgement. One connection holds at most
/// `connection + send`; a quarter of the budget per direction keeps two fully
/// saturated connections inside the budget on the smallest devices (64 MiB
/// budget: 16 MiB windows, still one full-speed stream), while larger budgets
/// reach the 64 MiB of four streams. Receive windows act per direction: this
/// side's downloads profit at once, its uploads once the peer is updated.
pub(super) fn transport_windows(budget: u64) -> TransportWindows {
    let quarter = u32::try_from(budget / 4).unwrap_or(u32::MAX);
    let connection = quarter.clamp(STREAM_RECEIVE_WINDOW, CONNECTION_RECEIVE_WINDOW);
    TransportWindows {
        stream: STREAM_RECEIVE_WINDOW.min(connection),
        connection,
        // More unacknowledged data than an updated peer grants per
        // connection could never be sent anyway.
        send: u64::from(connection),
    }
}

pub(super) fn iroh_transport_config() -> QuicTransportConfig {
    let windows = transport_windows(crate::transfer::memory_budget());
    QuicTransportConfig::builder()
        // Live idle peers remain open because the transport sends a keepalive
        // every five seconds. QUIC may extend its effective timeout to at least
        // three PTOs, so Exec adds its own authenticated application deadline.
        .max_idle_timeout(Some(
            VarInt::from_u32(IROH_CONNECTION_IDLE_TIMEOUT.as_millis() as u32).into(),
        ))
        .keep_alive_interval(IROH_CONNECTION_KEEPALIVE_INTERVAL)
        .default_path_keep_alive_interval(IROH_PATH_KEEPALIVE_INTERVAL)
        .max_concurrent_bidi_streams(VarInt::from_u32(IROH_MAX_CONCURRENT_BIDI_STREAMS))
        .max_concurrent_uni_streams(VarInt::from_u32(IROH_MAX_CONCURRENT_UNI_STREAMS))
        .stream_receive_window(VarInt::from_u32(windows.stream))
        .receive_window(VarInt::from_u32(windows.connection))
        .send_window(windows.send)
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
        assert_eq!(IROH_MAX_CONCURRENT_BIDI_STREAMS, 64);
        assert_eq!(IROH_MAX_CONCURRENT_UNI_STREAMS, 0);

        let _config = iroh_transport_config();
    }

    #[test]
    fn transfer_engine_task_quic_windows_cover_fast_long_links() {
        const MIB: u64 = 1024 * 1024;
        // One stream fills 1 Gbit/s at a 100 ms round trip.
        let bandwidth_delay = 1_000_000_000 / 8 / 10;
        for budget in [64 * MIB, 100 * MIB, 256 * MIB, 2048 * MIB] {
            let windows = transport_windows(budget);
            assert!(u64::from(windows.stream) >= bandwidth_delay, "{windows:?}");
            assert!(windows.stream <= windows.connection, "{windows:?}");
            assert_eq!(windows.send, u64::from(windows.connection));
            // Both directions of one connection stay within half the budget.
            assert!(
                u64::from(windows.connection) + windows.send <= budget / 2,
                "{windows:?}"
            );
        }
        assert_eq!(
            transport_windows(64 * MIB),
            TransportWindows {
                stream: 16 * MIB as u32,
                connection: 16 * MIB as u32,
                send: 16 * MIB,
            }
        );
        assert_eq!(transport_windows(2048 * MIB).connection, 64 * MIB as u32);
        assert_eq!(TRANSFER_STREAMS_PER_CONNECTION, 60);
        assert!(TRANSFER_STREAMS_PER_CONNECTION < IROH_MAX_CONCURRENT_BIDI_STREAMS);
    }
}
