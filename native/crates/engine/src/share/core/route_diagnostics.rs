//! Bounded facts, never endpoint addresses or authorization material.
use super::route_state::{RouteClass, RouteStatus};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RouteBudget {
    pub initial_pto_ms: u64,
    pub best_pto_ms: u64,
    pub collection_ms: u64,
    pub retry_intervals_ms: Vec<u64>,
    pub max_probe_attempts: u8,
    pub response_ms: u64,
    pub validation_ms: u64,
    pub direct_deadline_ms: u64,
    pub frozen_candidates: usize,
    /// Candidates the candidate cap refused in the last observation before the
    /// freeze; LAN paths are never refused. Absent (0) from older writers.
    #[serde(default)]
    pub dropped_candidates: usize,
}

pub fn milliseconds(value: Duration) -> u64 {
    u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteMeasurement {
    pub observed_at_unix_ms: u64,
    pub generation: u64,
    pub class: RouteClass,
    pub rtt_us: Option<u64>,
    /// Serialized STREAM/DATAGRAM payload, including retransmits and discarded I/O
    /// plans (TX), authenticated frame payload including repeats (RX). Not goodput.
    pub application_tx_bytes: u64,
    pub application_rx_bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteDiagnostics {
    pub connection_id: usize,
    pub protocol: String,
    pub status: RouteStatus,
    pub budget: RouteBudget,
    pub measurements: Vec<RouteMeasurement>,
    /// Direct retries of a connection whose relay carries payload as the
    /// marked fallback (`route_retry.rs`); absent otherwise and from older
    /// writers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<DirectRetryFacts>,
}

/// What started the last direct retry (`route_retry.rs`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryCause {
    /// The backoff timer (5 s, doubling up to 60 s).
    Backoff,
    /// This device's direct addresses changed.
    LocalAddresses,
    /// The peer published other direct addresses.
    PeerAddresses,
}

/// Retry facts of one connection in relay fallback, taken when read.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectRetryFacts {
    /// Direct attempts started since the relay became the fallback.
    pub attempts: u32,
    /// Time until the next attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_in_ms: Option<u64>,
    /// Wall-clock time of the next attempt (milliseconds since the Unix epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cause: Option<RetryCause>,
    /// Whether the last attempt started a NAT traversal round on the connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_nat_round: Option<bool>,
}

// E30 (M15): the cost of the share encryption against the share throughput.
#[cfg(test)]
#[path = "s4_messung_support.rs"]
mod s4_messung_support;

#[cfg(test)]
#[path = "s4_messung_task_tests.rs"]
mod s4_messung_task_tests;

// P7 (plan 2026-11-14, C7): the measurements M1 to M4, M8 and M9 of the
// connections per purpose; their support holds the loopback link.
#[cfg(test)]
#[path = "verbindungen_messung_support.rs"]
pub(crate) mod verbindungen_messung_support;

#[cfg(test)]
#[path = "verbindungen_messung_task_tests.rs"]
mod verbindungen_messung_task_tests;
