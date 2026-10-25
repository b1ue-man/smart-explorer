//! Bounded facts, never endpoint addresses or authorization material.
use std::time::Duration;
use serde::{Deserialize, Serialize};
use super::route_state::{RouteClass, RouteStatus};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct RouteBudget {
    pub(crate) initial_pto_ms: u64,
    pub(crate) best_pto_ms: u64,
    pub(crate) collection_ms: u64,
    pub(crate) retry_intervals_ms: Vec<u64>,
    pub(crate) max_probe_attempts: u8,
    pub(crate) response_ms: u64,
    pub(crate) validation_ms: u64,
    pub(crate) direct_deadline_ms: u64,
    pub(crate) frozen_candidates: usize,
}

pub(crate) fn milliseconds(value: Duration) -> u64 {
    u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RouteMeasurement {
    pub(crate) observed_at_unix_ms: u64,
    pub(crate) generation: u64,
    pub(crate) class: RouteClass,
    pub(crate) rtt_us: Option<u64>,
    /// Serialized STREAM/DATAGRAM payload, including retransmits and discarded I/O
    /// plans (TX), authenticated frame payload including repeats (RX). Not goodput.
    pub(crate) application_tx_bytes: u64,
    pub(crate) application_rx_bytes: u64,
    pub(crate) elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RouteDiagnostics {
    pub(crate) connection_id: usize,
    pub(crate) protocol: String,
    pub(crate) status: RouteStatus,
    pub(crate) budget: RouteBudget,
    pub(crate) measurements: Vec<RouteMeasurement>,
}
