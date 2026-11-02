//! Start phase benchmark (Z-R2): time to first byte and payload bytes over
//! relay versus LAN/direct, over a private Iroh relay on loopback with the
//! production route monitor. Prints one JSON line per scenario:
//! `cargo test --release -- --ignored speed_task_share_start_phase --nocapture --test-threads=1`.
use std::time::Duration;

use super::super::super::route_monitor;
use super::super::super::route_state::{RouteClass, RoutePhase};
use super::payload_bytes_by_class;
use super::task_tests::{
    assert_no_relay_after_direct, drive, relay_tx, runtime, trace, Pair, CLIENT, FILE,
};

/// 32 MiB in 128 files over 8 request lanes.
const FILES: u64 = 128;
const LANES: usize = 8;

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
