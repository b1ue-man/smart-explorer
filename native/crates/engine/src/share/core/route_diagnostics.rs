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
