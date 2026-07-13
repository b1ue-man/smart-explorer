use std::time::Duration;

/// Authenticated Exec heartbeats are separate from QUIC keepalives. QUIC's
/// effective idle timeout may grow to at least three probe timeouts, while an
/// abandoned remote shell must reach a terminal state within a fixed bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExecHeartbeatPolicy {
    pub(crate) interval: Duration,
    pub(crate) peer_timeout: Duration,
    pub(crate) write_timeout: Duration,
}

impl ExecHeartbeatPolicy {
    /// Server budget from a completed terminal-frame write until ResultAck.
    /// The client may still spend one peer-liveness window delivering an
    /// earlier event and one bounded write on ResultAck. A heartbeat interval
    /// is strict scheduling slack beyond that latest valid acknowledgement.
    pub(crate) fn server_result_ack_timeout(self) -> Duration {
        self.peer_timeout
            .saturating_add(self.write_timeout)
            .saturating_add(self.interval)
    }

    /// Client budget from receiving Terminal/Error until ResultAcknowledged.
    /// The server may consume its complete ResultAck receive budget and one
    /// bounded confirmation write. Another heartbeat interval remains as
    /// strict scheduling slack, so client and server never expire together.
    pub(crate) fn client_result_acknowledged_timeout(self) -> Duration {
        self.server_result_ack_timeout()
            .saturating_add(self.write_timeout)
            .saturating_add(self.interval)
    }
}

pub(crate) const EXEC_HEARTBEAT_POLICY: ExecHeartbeatPolicy = ExecHeartbeatPolicy {
    interval: Duration::from_secs(5),
    peer_timeout: Duration::from_secs(20),
    write_timeout: Duration::from_secs(10),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_allows_multiple_missed_heartbeats_but_bounds_writes() {
        assert!(EXEC_HEARTBEAT_POLICY.interval > Duration::ZERO);
        assert!(EXEC_HEARTBEAT_POLICY.peer_timeout >= EXEC_HEARTBEAT_POLICY.interval * 3);
        assert!(EXEC_HEARTBEAT_POLICY.write_timeout < EXEC_HEARTBEAT_POLICY.peer_timeout);

        let latest_client_ack = EXEC_HEARTBEAT_POLICY
            .peer_timeout
            .saturating_add(EXEC_HEARTBEAT_POLICY.write_timeout);
        let server_budget = EXEC_HEARTBEAT_POLICY.server_result_ack_timeout();
        assert!(server_budget > latest_client_ack);
        assert_eq!(
            server_budget - latest_client_ack,
            EXEC_HEARTBEAT_POLICY.interval
        );
        let latest_server_confirmation =
            server_budget.saturating_add(EXEC_HEARTBEAT_POLICY.write_timeout);
        let client_budget = EXEC_HEARTBEAT_POLICY.client_result_acknowledged_timeout();
        assert!(client_budget > latest_server_confirmation);
        assert_eq!(
            client_budget - latest_server_confirmation,
            EXEC_HEARTBEAT_POLICY.interval
        );
    }
}
