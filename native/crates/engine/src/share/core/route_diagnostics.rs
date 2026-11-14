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
