//! V7 oracles over observations from real relay IO; never a transport simulator.
use serde::Serialize;
use serde_json::{json, Value};

pub(super) const FPS: u32 = 30;
pub(super) const FRAME_US: u64 = 1_000_000 / FPS as u64;
pub(super) const WARMUP_US: u64 = 5_000_000;
pub(super) const STATIONARY_US: u64 = 12_000_000;

#[derive(Clone, Copy, Serialize)]
pub(super) struct Profile {
    pub label: &'static str,
    pub bps: u64,
    pub windows: usize,
}

#[derive(Clone, Serialize)]
pub(super) struct Delivery {
    pub flow: usize,
    pub at_us: u64,
    pub bytes: u64,
    pub captured_us: Option<u64>,
}

#[derive(Clone, Serialize)]
pub(super) struct Offer {
    pub flow: usize,
    pub at_us: u64,
    pub target_bps: u64,
    pub bytes: u64,
    pub prepare_us: u64,
    pub interval_us: u64,
    pub rtt_us: u64,
}

#[derive(Clone, Serialize)]
pub(super) struct ProbeSent {
    pub flow: usize,
    pub sequence: u64,
    pub at_us: u64,
}

#[derive(Clone, Serialize)]
pub(super) struct Control {
    pub flow: usize,
    pub sequence: u64,
    pub sent_us: u64,
    pub received_us: u64,
}

#[derive(Clone, Default, Serialize)]
pub(super) struct Observed {
    pub deliveries: Vec<Delivery>,
    pub offers: Vec<Offer>,
    pub probes_sent: Vec<ProbeSent>,
    pub controls: Vec<Control>,
    pub broken_streams: Vec<(usize, u64, u64)>,
}

#[derive(Serialize)]
pub(super) struct Measurement {
    pub start_us: u64,
    pub stationary_start_us: u64,
    pub end_us: u64,
    pub idle_rtt_us: u64,
    pub observed: Observed,
}

fn inside(at: u64, sample: &Measurement) -> bool {
    (sample.stationary_start_us..sample.end_us).contains(&at)
}

fn percentile95(values: &mut [u64]) -> Option<u64> {
    values.sort_unstable();
    values
        .get((values.len() * 95).div_ceil(100).checked_sub(1)?)
        .copied()
}

fn throughput(sample: &Measurement, flow: Option<usize>) -> f64 {
    let bytes: u64 = sample
        .observed
        .deliveries
        .iter()
        .filter(|d| inside(d.at_us, sample) && flow.is_none_or(|f| d.flow == f))
        .map(|d| d.bytes)
        .sum();
    bytes as f64 * 8_000_000.0 / STATIONARY_US as f64
}

fn control_p95(
    sample: &Measurement,
    flow: usize,
    offers: &[Offer],
    failures: &mut Vec<String>,
) -> (u64, u64) {
    let sent: Vec<_> = sample
        .observed
        .probes_sent
        .iter()
        .filter(|probe| probe.flow == flow && inside(probe.at_us, sample))
        .collect();
    let mut latencies = Vec::new();
    let mut adjusted = Vec::new();
    for probe in &sent {
        let received: Vec<_> = sample
            .observed
            .controls
            .iter()
            .filter(|reply| {
                reply.flow == flow
                    && reply.sequence == probe.sequence
                    && reply.sent_us == probe.at_us
            })
            .collect();
        if received.len() != 1 || received[0].received_us < probe.at_us {
            failures.push(format!(
                "flow {flow}: missing, duplicate or clock-invalid control reply {}",
                probe.sequence
            ));
        } else {
            let latency = received[0].received_us - probe.at_us;
            let frame_us = offers
                .iter()
                .rev()
                .find(|offer| offer.flow == flow && offer.at_us <= probe.at_us)
                .map_or(FRAME_US, |offer| offer.interval_us);
            latencies.push(latency);
            adjusted.push(latency.saturating_sub(2 * frame_us));
        }
    }
    if sent.len() < 100 {
        failures.push(format!(
            "flow {flow}: insufficient stationary control samples: {}",
            sent.len()
        ));
    }
    (
        percentile95(&mut latencies).unwrap_or(u64::MAX),
        percentile95(&mut adjusted).unwrap_or(u64::MAX),
    )
}

fn queue_trend(sample: &Measurement, flow: usize, failures: &mut Vec<String>) -> Value {
    let mut buckets = Vec::new();
    for index in 0..STATIONARY_US / 1_000_000 {
        let begin = sample.stationary_start_us + index * 1_000_000;
        let mut ages: Vec<_> = sample
            .observed
            .deliveries
            .iter()
            .filter(|d| d.flow == flow && (begin..begin + 1_000_000).contains(&d.at_us))
            .filter_map(|d| d.captured_us.and_then(|born| d.at_us.checked_sub(born)))
            .collect();
        if ages.is_empty() {
            failures.push(format!(
                "flow {flow}: no complete received frame in stationary second {index}"
            ));
            return json!({"buckets_us": buckets, "valid": false});
        }
        buckets.push(percentile95(&mut ages).unwrap());
    }
    let mut score = 0i64;
    for (index, value) in buckets.iter().enumerate() {
        for next in &buckets[index + 1..] {
            score += match next.cmp(value) {
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
            };
        }
    }
    let n = buckets.len() as f64;
    let mut ties = std::collections::BTreeMap::<u64, u64>::new();
    for value in &buckets {
        *ties.entry(*value).or_default() += 1;
    }
    let correction: f64 = ties
        .values()
        .map(|&count| {
            let t = count as f64;
            t * (t - 1.0) * (2.0 * t + 5.0)
        })
        .sum();
    let variance = (n * (n - 1.0) * (2.0 * n + 5.0) - correction) / 18.0;
    let z = if score > 0 && variance > 0.0 {
        (score - 1) as f64 / variance.sqrt()
    } else {
        0.0
    };
    let monotone = buckets.windows(2).all(|pair| pair[1] > pair[0]);
    if monotone || z > 1.6448536269514722 {
        failures.push(format!(
            "flow {flow}: stationary queue age keeps growing (Mann-Kendall z={z:.3})"
        ));
    }
    json!({"buckets_us": buckets, "mann_kendall_score": score, "positive_z": z,
        "strictly_increasing": monotone, "valid": true})
}

pub(super) fn evaluate(profile: Profile, reference: &Measurement, screen: &Measurement) -> Value {
    let mut failures = Vec::new();
    for (label, sample) in [("reference", reference), ("screen", screen)] {
        if sample.end_us.checked_sub(sample.stationary_start_us) != Some(STATIONARY_US) {
            failures.push(format!(
                "{label}: stationary window differs from the fixed observation period"
            ));
        }
        if !(18_000..=35_000).contains(&sample.idle_rtt_us) {
            failures.push(format!(
                "{label}: observed unloaded RTT {} us is inconsistent with the 20-ms profile",
                sample.idle_rtt_us
            ));
        }
    }
    let r_ref = throughput(reference, None);
    let r_screen = throughput(screen, None);
    if r_ref < profile.bps as f64 * 0.8 || r_ref > profile.bps as f64 * 1.05 {
        failures.push(format!(
            "reference is not a saturated {}-bit/s path: {r_ref:.0} bit/s",
            profile.bps
        ));
    }
    if r_screen < r_ref * 0.9 {
        failures.push(format!(
            "stationary useful throughput {r_screen:.0} < 90% of measured reference {r_ref:.0}"
        ));
    }
    let r_start = super::super::START_RELAY_BPS * profile.windows as u64;
    let intervals = ((r_ref / r_start as f64).max(1.0).ln() / 1.25f64.ln()).ceil() as u64 + 2;
    let mut interval = screen.idle_rtt_us.max(FRAME_US);
    let mut targets = vec![super::super::START_RELAY_BPS; profile.windows];
    let mut rtts = vec![screen.idle_rtt_us; profile.windows];
    let mut frame_intervals = vec![FRAME_US; profile.windows];
    let mut first = vec![None; profile.windows];
    let mut offers = screen.observed.offers.clone();
    offers.sort_by_key(|offer| offer.at_us);
    let origin = offers.first().map_or(screen.start_us, |offer| offer.at_us);
    let mut reached = (r_start as f64 >= r_ref).then_some(0);
    let mut reached_intervals = reached.map(|_| 0.0);
    let mut previous = origin;
    let mut elapsed_intervals = 0.0;
    let mut prepared_bytes = 0u128;
    let mut preparation_us = 0u128;
    for offer in &offers {
        if offer.flow >= profile.windows {
            failures.push("offer has an unknown window identity".to_owned());
            continue;
        }
        elapsed_intervals += offer.at_us.saturating_sub(previous) as f64 / interval as f64;
        previous = offer.at_us;
        first[offer.flow].get_or_insert(offer.target_bps);
        targets[offer.flow] = offer.target_bps;
        rtts[offer.flow] = offer.rtt_us;
        frame_intervals[offer.flow] = offer.interval_us;
        interval = rtts
            .iter()
            .chain(&frame_intervals)
            .copied()
            .max()
            .unwrap_or(1)
            .max(1);
        if targets.iter().sum::<u64>() as f64 >= r_ref && reached.is_none() {
            reached = offer.at_us.checked_sub(origin);
            reached_intervals = Some(elapsed_intervals);
        }
        if offer.interval_us == 0 || offer.prepare_us == 0 {
            failures.push(format!(
                "flow {}: synthetic offer has invalid observed timing",
                offer.flow
            ));
        }
        prepared_bytes += u128::from(offer.bytes);
        preparation_us += u128::from(offer.prepare_us);
    }
    let supply_bps = prepared_bytes * 8 * 1_000_000 / preparation_us.max(1);
    if supply_bps < u128::from(profile.bps) {
        failures.push(
            "actual synthetic frame preparation cannot saturate the profiled link".to_owned(),
        );
    }
    if first
        .iter()
        .any(|value| *value != Some(super::super::START_RELAY_BPS))
    {
        failures
            .push("not every window exposed the actual conservative relay start target".to_owned());
    }
    if reached_intervals.is_none_or(|elapsed| elapsed > intervals as f64) {
        failures.push(format!(
            "target did not reach measured reference within {intervals} measured control intervals"
        ));
    }
    let mut windows = Vec::new();
    for flow in 0..profile.windows {
        let frame_interval = offers
            .iter()
            .filter(|offer| offer.flow == flow && inside(offer.at_us, screen))
            .map(|offer| offer.interval_us)
            .max()
            .unwrap_or(FRAME_US);
        let (ref_latency, _) = control_p95(reference, flow, &[], &mut failures);
        let (latency, budgeted_latency) = control_p95(screen, flow, &offers, &mut failures);
        if budgeted_latency > ref_latency {
            failures.push(format!(
                "flow {flow}: P95 control latency exceeds reference + two frame intervals"
            ));
        }
        let rate = throughput(screen, Some(flow));
        if rate < r_ref / profile.windows as f64 * 0.5 {
            failures.push(format!(
                "flow {flow}: shared peer budget starves a visible window"
            ));
        }
        windows.push(json!({"flow": flow, "useful_bps": rate,
            "stationary_broken_streams": screen.observed.broken_streams.iter()
                .filter(|(window, _, at)| *window == flow && inside(*at, screen)).count(),
            "observed_frame_interval_us": frame_interval,
            "reference_control_p95_us": ref_latency, "screen_control_p95_us": latency,
            "screen_control_p95_after_two_actual_frame_intervals_us": budgeted_latency,
            "queue": queue_trend(screen, flow, &mut failures)}));
    }
    json!({"profile": profile, "reference_useful_bps": r_ref, "screen_useful_bps": r_screen,
        "start_bps": r_start, "ramp_allowed_intervals": intervals,
        "synthetic_prepare_capacity_bps": supply_bps,
        "ramp_timing": "integral of elapsed time / max(observed RTT, observed frame interval) across the peer's windows",
        "ramp_reached_control_intervals": reached_intervals, "ramp_reached_us": reached,
        "stationary_us": STATIONARY_US, "windows": windows,
        "result": if failures.is_empty() {"passed"} else {"failed"}, "failures": failures})
}
