//! When `peer_feed` starts, keeps, pauses or leaves a device's feed session
//! (A4, A5 of plan 2026-11-06-akku-rechte). The facts come from one look of
//! `observe` at the device; the decision itself has no clock and no I/O.
//!
//! - A running feed restarts when the service or the relation target
//!   changed; in low-power operation it pauses unless a mapped service of
//!   the device is in use (a listener the person opened or a tunnel).
//! - A paused feed starts again once this device no longer runs in
//!   low-power operation.
//! - A refused feed (rights, „Nachfragen“, older version, invalid update)
//!   starts again when this device's relation to that device changed
//!   (contact, grant, room membership; presence and routes do not count),
//!   when this app becomes visible, or after its refusal backoff
//!   (`refused_backoff`): the other device's user may have allowed this one
//!   meanwhile, and that change reaches this device by no event of its own.
//! - A lost feed (connection, unreachable device, thread start) starts again
//!   at once on an arrival of that device or this service's server
//!   connection, otherwise after its backoff. A feed the host ended as idle
//!   in its low-power operation (`host_rested`) is no loss: presence events
//!   of that device, which a resting phone still sends every few minutes,
//!   do not restart it; only this service's server connection, a relation
//!   change, this app becoming visible or `RESTED_RETRY` do.
use std::time::Duration;

/// Backoff of the first retry after a lost feed: one QUIC keepalive
/// interval (`share/core/keepalive.rs`), so a connection that dropped
/// together with a short network change comes back without a visible gap.
const FIRST_RETRY: Duration = Duration::from_secs(5);
/// Longest backoff. Every reconnection costs two connections with a hybrid
/// post-quantum handshake on both devices; a device that dropped the feed
/// without any event of its own (no presence, no LAN sighting) is asleep
/// or unreachable, and any such event retries at once anyway.
const LONGEST_RETRY: Duration = Duration::from_secs(10 * 60);
/// Wait before a feed the host ended as idle starts again: the host closes a
/// quiet feed after two quiet minutes of low-power operation, so a sooner
/// restart would wake the resting device in a loop.
pub(super) const RESTED_RETRY: Duration = LONGEST_RETRY;
/// First retry after a refusal: long enough that a refusing host is not asked
/// again at once, short enough that a right just allowed there shows soon.
const FIRST_REFUSED_RETRY: Duration = Duration::from_secs(60);
/// Longest wait between two retries of a refused feed: at most two probe
/// handshakes an hour on the refusing device.
const LONGEST_REFUSED_RETRY: Duration = Duration::from_secs(30 * 60);

/// How a device's feed session stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Running,
    /// Ended by this device's low-power operation; no connection.
    Paused,
    /// Ended by the host's answer; a relation change, the app or the
    /// refusal backoff retries.
    Refused,
    /// Ended by the connection or a failed start; retried with backoff.
    Lost,
}

/// What `observe` knows about one present device with a session.
#[derive(Clone, Copy, Debug)]
pub(super) struct Facts {
    pub(super) phase: Phase,
    /// Same Share service and same relation target as the session.
    pub(super) same: bool,
    pub(super) low_power: bool,
    /// A mapped service of the device has a listener or a tunnel.
    pub(super) tunnel_in_use: bool,
    pub(super) relation_changed: bool,
    /// The device arrived (presence, room, LAN) or the server connected.
    pub(super) device_event: bool,
    pub(super) backoff_over: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Keep,
    Start,
    Pause,
}

/// A device without a session: start it, or note it paused.
pub(super) fn first(low_power: bool) -> Step {
    if low_power {
        Step::Pause
    } else {
        Step::Start
    }
}

pub(super) fn step(facts: &Facts) -> Step {
    let connect = !facts.low_power || facts.tunnel_in_use;
    let renew = if connect { Step::Start } else { Step::Pause };
    match facts.phase {
        Phase::Running if !facts.same => renew,
        Phase::Running if !connect => Step::Pause,
        Phase::Running => Step::Keep,
        Phase::Paused if !facts.low_power => Step::Start,
        Phase::Paused => Step::Keep,
        Phase::Refused if !facts.same || facts.relation_changed || facts.backoff_over => renew,
        Phase::Refused => Step::Keep,
        Phase::Lost if !facts.same || facts.device_event || facts.backoff_over => renew,
        Phase::Lost => Step::Keep,
    }
}

/// The wait before the next attempt after `failures` lost attempts in a
/// row that never reached the subscription: doubling, bounded.
pub(super) fn backoff(failures: u32) -> Duration {
    let factor = 1u32.checked_shl(failures).unwrap_or(u32::MAX);
    FIRST_RETRY.saturating_mul(factor).min(LONGEST_RETRY)
}

/// The wait before the next attempt after `refusals` refused attempts in a
/// row before the last one: doubling from one minute, bounded.
pub(super) fn refused_backoff(refusals: u32) -> Duration {
    let factor = 1u32.checked_shl(refusals).unwrap_or(u32::MAX);
    FIRST_REFUSED_RETRY
        .saturating_mul(factor)
        .min(LONGEST_REFUSED_RETRY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(phase: Phase) -> Facts {
        Facts {
            phase,
            same: true,
            low_power: false,
            tunnel_in_use: false,
            relation_changed: false,
            device_event: false,
            backoff_over: false,
        }
    }

    #[test]
    fn akku_task_low_power_holds_no_feed_connection_without_a_tunnel_in_use() {
        let cases: [(&str, fn()); 4] = [
            (
                "akku_task_low_power_holds_no_feed_connection_without_a_tunnel_in_use",
                akku_task_low_power_holds_no_feed_connection_without_a_tunnel_in_use_case,
            ),
            (
                "akku_task_lost_feed_backs_off_and_an_event_of_the_device_retries_at_once",
                akku_task_lost_feed_backs_off_and_an_event_of_the_device_retries_at_once,
            ),
            (
                "akku_task_refused_feed_waits_for_a_relation_change_of_that_device",
                akku_task_refused_feed_waits_for_a_relation_change_of_that_device,
            ),
            (
                "akku_task_running_feed_follows_a_changed_target_or_service",
                akku_task_running_feed_follows_a_changed_target_or_service,
            ),
        ];
        for (case, check) in cases {
            std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
        }
    }

    fn akku_task_low_power_holds_no_feed_connection_without_a_tunnel_in_use_case() {
        let running = Facts {
            low_power: true,
            ..facts(Phase::Running)
        };
        assert_eq!(step(&running), Step::Pause);
        let tunnel = Facts {
            tunnel_in_use: true,
            ..running
        };
        assert_eq!(step(&tunnel), Step::Keep);
        let paused = Facts {
            low_power: true,
            device_event: true,
            relation_changed: true,
            ..facts(Phase::Paused)
        };
        assert_eq!(step(&paused), Step::Keep);
        let visible = Facts {
            low_power: false,
            ..paused
        };
        assert_eq!(step(&visible), Step::Start);
        let lost = Facts {
            low_power: true,
            device_event: true,
            ..facts(Phase::Lost)
        };
        assert_eq!(step(&lost), Step::Pause);
        assert_eq!(first(true), Step::Pause);
        assert_eq!(first(false), Step::Start);
    }

    fn akku_task_lost_feed_backs_off_and_an_event_of_the_device_retries_at_once() {
        let lost = facts(Phase::Lost);
        assert_eq!(step(&lost), Step::Keep);
        let due = Facts {
            backoff_over: true,
            ..lost
        };
        assert_eq!(step(&due), Step::Start);
        let arrived = Facts {
            device_event: true,
            ..lost
        };
        assert_eq!(step(&arrived), Step::Start);
        assert_eq!(backoff(0), Duration::from_secs(5));
        assert_eq!(backoff(1), Duration::from_secs(10));
        assert_eq!(backoff(3), Duration::from_secs(40));
        assert_eq!(backoff(10), LONGEST_RETRY);
        assert_eq!(backoff(u32::MAX), LONGEST_RETRY);
    }

    fn akku_task_refused_feed_waits_for_a_relation_change_of_that_device() {
        let refused = facts(Phase::Refused);
        assert_eq!(step(&refused), Step::Keep);
        // Presence and arrivals are no reason for a refused feed.
        let arrived = Facts {
            device_event: true,
            ..refused
        };
        assert_eq!(step(&arrived), Step::Keep);
        let changed = Facts {
            relation_changed: true,
            ..refused
        };
        assert_eq!(step(&changed), Step::Start);
        // The other device's user may have allowed this one since: after its
        // refusal backoff the feed asks once more (rights change unseen here).
        let waited = Facts {
            backoff_over: true,
            ..refused
        };
        assert_eq!(step(&waited), Step::Start);
        assert_eq!(refused_backoff(0), Duration::from_secs(60));
        assert_eq!(refused_backoff(1), Duration::from_secs(120));
        assert_eq!(refused_backoff(10), LONGEST_REFUSED_RETRY);
        assert_eq!(refused_backoff(u32::MAX), LONGEST_REFUSED_RETRY);
        let low_power = Facts {
            low_power: true,
            ..changed
        };
        assert_eq!(step(&low_power), Step::Pause);
    }

    fn akku_task_running_feed_follows_a_changed_target_or_service() {
        let moved = Facts {
            same: false,
            ..facts(Phase::Running)
        };
        assert_eq!(step(&moved), Step::Start);
        assert_eq!(step(&facts(Phase::Running)), Step::Keep);
    }
}
