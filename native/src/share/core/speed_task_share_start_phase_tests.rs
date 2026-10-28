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

#[test]
#[ignore = "benchmark: --ignored speed_task_share_start_phase --nocapture"]
fn speed_task_share_start_phase_bench() {
    runtime().block_on(async {
        for (scenario, direct) in [("relay_then_direct", true), ("relay_only", false)] {
            let relay = iroh::test_utils::run_relay_server_with(false).await;
            let (map, url, _relay) = relay.expect("private relay");
            let pair = Pair::new(&map, &url, direct, true).await;
            let seen = trace(&pair.client, pair.started);
            let sent = drive(&pair.client, LANES, |done| done >= FILES).await;
            let elapsed = pair.started.elapsed();
            pair.ledger.assert_exactly_once(sent, FILE);
            let ttfb = pair
                .ledger
                .first_byte()
                .expect("host received payload")
                .saturating_duration_since(pair.started);
            let seen = seen.lock().unwrap().clone();
            let relay_start_ms = seen
                .iter()
                .find(|seen| seen.phase == RoutePhase::RelayStart)
                .map(|seen| ms(seen.at));
            let switch = assert_no_relay_after_direct(&seen, relay_tx(&pair.client));
            let switch_class = match switch.as_ref().and_then(|seen| seen.class) {
                Some(RouteClass::Lan) => Some("lan"),
                Some(RouteClass::Direct) => Some("direct"),
                Some(RouteClass::Relay) | None => None,
            };
            let remote = pair.client.remote_id().to_string();
            let budget = route_monitor::diagnostics(CLIENT, Some(&remote))
                .into_iter()
                .next()
                .map(|facts| facts.budget)
                .unwrap_or_default();
            let (client_relay, client_direct) = payload_bytes_by_class(&pair.client);
            let (host_relay, host_direct) = payload_bytes_by_class(&pair.host);
            let bytes = sent * FILE as u64;
            let report = serde_json::json!({
                "scenario": scenario,
                "files": sent,
                "bytes": bytes,
                "lanes": LANES,
                "time_to_first_byte_ms": ms(ttfb),
                "relay_start_admitted_ms": relay_start_ms,
                "first_lan_or_direct_ms": switch.as_ref().map(|seen| ms(seen.at)),
                "first_lan_or_direct_class": switch_class,
                // Before Z-R2 relay payload could start only after this offset.
                "direct_deadline_ms": budget.direct_deadline_ms,
                "elapsed_ms": ms(elapsed),
                "mib_per_s": bytes as f64 / 1_048_576.0 / elapsed.as_secs_f64().max(0.001),
                "client_payload_bytes_relay": client_relay,
                "client_payload_bytes_lan_or_direct": client_direct,
                "host_payload_bytes_relay": host_relay,
                "host_payload_bytes_lan_or_direct": host_direct,
                "route_changes": seen.len(),
                "max_pause_epoch": seen.iter().map(|seen| seen.pause_epoch).max(),
            });
            println!("SE_SPEED_TASK_SHARE_START_PHASE={report}");
            if !direct {
                assert!(switch.is_none(), "relay-only pair has no direct path");
                assert!(relay_start_ms.is_some(), "{seen:?}");
            }
            pair.close().await;
        }
    });
}
