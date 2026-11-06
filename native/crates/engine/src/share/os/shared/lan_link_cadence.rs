//! Cadence of the paired LAN status rounds.
//!
//! While the app is visible (and on every desktop) the rounds keep their
//! original rhythm: one round every 2 s inside sessions of 45 s. While the
//! host reports low-power operation (Android in the background, see
//! `share::power`) every round costs radio and CPU wake-ups for nothing the
//! user can see: a session that is still open then rests 30 s between
//! rounds until it ends, and an accepting peer waits as long for the next
//! round. A fact keeps its 6-s validity in every rhythm: the facts authorize
//! short uplink leases of the privileged worker
//! (`lan_uplink_evidence::authorize`), so a slower rhythm leaves them absent
//! between rounds (uplink fails closed) instead of stretching their freshness.
//!
//! A device in low-power operation keeps no session at all, in neither
//! direction: every open QUIC connection sends a keepalive every 5 s
//! (`share/core/keepalive.rs`), so a session of any rhythm kept the phone's
//! radio awake, and a peer that dials it chose its own 2-s rhythm. It ends
//! its sessions when low power begins, dials nobody, and closes an inbound
//! session at once with `RESTING_CLOSE_CODE`; a dialling peer of this
//! version then leaves it alone for `RESTING_COOLDOWN` (the resting device
//! dials itself once it is visible again), an older one after its cooldown.
//! The facts only authorize short uplink leases, which fail closed meanwhile.
use super::*;
use iroh::endpoint::ConnectionError;

/// Application close code of a session a resting device does not keep ("RS").
const RESTING_CLOSE_CODE: u32 = 0x5253;
/// How long a dialling device leaves a resting peer alone.
pub(super) const RESTING_COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// Gap between two rounds while the host is in low-power operation.
const LOW_POWER_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// Session limit while the host is in low-power operation: the same count of
/// rounds (22) as the 45 s of the visible rhythm at 2 s.
const LOW_POWER_SESSION_DEADLINE: Duration = Duration::from_secs(22 * 30);
/// An inbound session ends when the dialling peer ends it or after the
/// longest session any peer may keep; the dialling side picks the rhythm.
pub(crate) const SERVER_SESSION_DEADLINE: Duration = LOW_POWER_SESSION_DEADLINE;
/// Failed dials of an absent peer are retried at this distance in low power.
const LOW_POWER_PROBE_COOLDOWN: Duration = Duration::from_secs(300);

fn refresh_interval(low_power: bool) -> Duration {
    if low_power {
        LOW_POWER_REFRESH_INTERVAL
    } else {
        REFRESH_INTERVAL
    }
}

pub(crate) fn session_deadline(low_power: bool) -> Duration {
    if low_power {
        LOW_POWER_SESSION_DEADLINE
    } else {
        SESSION_DEADLINE
    }
}

/// Longest time a server waits for the next round of a peer: the stretched
/// rhythm of a peer in low power plus the budget of the round itself.
pub(crate) fn server_idle_wait() -> Duration {
    ROUND_DEADLINE + LOW_POWER_REFRESH_INTERVAL
}

/// Distance between two dials of one peer that did not end in a session.
pub(super) fn probe_cooldown(low_power: bool) -> Duration {
    if low_power {
        LOW_POWER_PROBE_COOLDOWN
    } else {
        SESSION_DEADLINE + PROBE_BACKOFF
    }
}

impl Cached {
    /// How long this confirmation may be consumed: the same 6 s in every
    /// rhythm (module text).
    pub(super) fn validity(&self) -> Duration {
        FACT_MONOTONIC_TTL
    }

    /// The fact as consumers see it: as confirmed, never re-dated.
    pub(super) fn lease(&self, _now: i64) -> AuthenticatedLanFact {
        self.fact.clone()
    }
}

impl LanLinkTransport {
    /// Remembers the host's power mode; a change ends the running rests so
    /// the next round follows the new rhythm at once, and low power ends
    /// every session (module text).
    pub(super) fn note_power(&self, low_power: bool) {
        if self.low_power.swap(low_power, Ordering::AcqRel) != low_power {
            self.power_changed.notify_waiters();
            if low_power {
                if let Ok(state) = self.state.lock() {
                    for channel in state.channels.values() {
                        rest(&channel.connection);
                    }
                }
            }
        }
    }

    /// This device rests in low-power operation: no session, no dial.
    pub(super) fn resting(&self) -> bool {
        self.low_power.load(Ordering::Acquire)
    }

    /// After a dialled session: the peer ended it because it rests.
    pub(super) fn note_peer_rest(&self, connection: &Connection, peer: String) {
        let rested = matches!(
            connection.close_reason(),
            Some(ConnectionError::ApplicationClosed(close))
                if close.error_code == VarInt::from_u32(RESTING_CLOSE_CODE)
        );
        if rested {
            if let Ok(mut state) = self.state.lock() {
                state.rested.insert(peer, Instant::now());
            }
        }
    }

    /// The client's pause between two rounds.
    pub(crate) async fn rest(&self, _connection: &Connection, low_power: bool) {
        let interval = refresh_interval(low_power);
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = self.power_changed.notified() => {}
        }
    }
}

/// Ends `connection` as a resting device.
pub(super) fn rest(connection: &Connection) {
    connection.close(
        VarInt::from_u32(RESTING_CLOSE_CODE),
        b"paired link rests in low power",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_task_low_power_stretches_rounds_and_sessions() {
        assert_eq!(refresh_interval(false), Duration::from_secs(2));
        assert_eq!(refresh_interval(true), Duration::from_secs(30));
        assert_eq!(session_deadline(false), Duration::from_secs(45));
        assert_eq!(session_deadline(true), Duration::from_secs(660));
        // The accepting side outlasts the longest rhythm plus its own round.
        assert!(server_idle_wait() >= LOW_POWER_REFRESH_INTERVAL + ROUND_DEADLINE);
        assert!(SERVER_SESSION_DEADLINE >= session_deadline(true));
        // A failed dial of an absent peer is retried more rarely in low power.
        assert_eq!(probe_cooldown(false), Duration::from_secs(50));
        assert!(probe_cooldown(true) > probe_cooldown(false));
        // Facts keep their short validity in every rhythm.
        assert!(FACT_MONOTONIC_TTL < LOW_POWER_REFRESH_INTERVAL);
    }
}
