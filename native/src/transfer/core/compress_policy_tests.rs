//! The LZ4 decision of Share and agent streams: when it compresses, how long
//! a decision holds and when the ratio is sampled.
use std::time::{Duration, Instant};

use super::compress_policy::{
    Policy, PolicyInput, METER_WINDOW, PROBE_BYTES, PROBE_PERIOD, WINDOW_BLOCKS,
};

const MIB: u64 = 1024 * 1024;

/// A wire-limited stream of 12.5 MB/s with a core that compresses text.
fn compressible() -> PolicyInput {
    PolicyInput {
        writer_blocked_share: 0.9,
        wire_bps: 12_500_000,
        core_bps: 350_000_000,
        ratio: 0.3,
        free_cores: 3.0,
    }
}

#[test]
fn speed_task_lz4_policy_compresses_when_core_rate_exceeds_link_by_thirty_percent() {
    let mut policy = Policy::new();
    assert!(policy.should_compress(compressible()));

    // 1.3 × the wire rate is the line: 12_999_999 is below it, 13_000_000 on it.
    let wire = 10_000_000;
    let at = |core_bps| PolicyInput {
        wire_bps: wire,
        core_bps,
        ..compressible()
    };
    assert!(!Policy::new().should_compress(at(12_999_999)));
    assert!(Policy::new().should_compress(at(13_000_000)));
    assert!(Policy::new().should_compress(at(13_000_001)));
}

#[test]
fn speed_task_lz4_policy_skips_when_the_writer_is_not_blocked() {
    let idle = PolicyInput {
        writer_blocked_share: 0.5,
        ..compressible()
    };
    assert!(
        !Policy::new().should_compress(idle),
        "exactly half is not a majority: the wire does not limit"
    );
    let free_flowing = PolicyInput {
        writer_blocked_share: 0.0,
        ..compressible()
    };
    assert!(!Policy::new().should_compress(free_flowing));
    let waiting = PolicyInput {
        writer_blocked_share: 0.51,
        ..compressible()
    };
    assert!(Policy::new().should_compress(waiting));
}

#[test]
fn speed_task_lz4_policy_skips_when_ratio_is_above_085() {
    let barely = PolicyInput {
        ratio: 0.85,
        ..compressible()
    };
    assert!(Policy::new().should_compress(barely), "0.85 is allowed");
    let random = PolicyInput {
        ratio: 0.851,
        ..compressible()
    };
    assert!(!Policy::new().should_compress(random));
    let incompressible = PolicyInput {
        ratio: 1.0,
        ..compressible()
    };
    assert!(!Policy::new().should_compress(incompressible));
}

#[test]
fn speed_task_lz4_policy_needs_a_free_core() {
    let busy = PolicyInput {
        free_cores: 0.99,
        ..compressible()
    };
    assert!(!Policy::new().should_compress(busy));
    let one = PolicyInput {
        free_cores: 1.0,
        ..compressible()
    };
    assert!(Policy::new().should_compress(one));
}

#[test]
fn speed_task_lz4_policy_keeps_its_decision_for_eight_blocks() {
    let bad = PolicyInput {
        ratio: 1.0,
        ..compressible()
    };
    let mut policy = Policy::new();
    assert!(policy.should_compress(compressible()), "block 1 decides");
    for block in 2..=WINDOW_BLOCKS {
        assert!(
            policy.should_compress(bad),
            "block {block} still holds the decision"
        );
    }
    assert!(
        !policy.should_compress(bad),
        "block {} decides again",
        WINDOW_BLOCKS + 1
    );
    // And the other way round: a refusal holds as well.
    for _ in 2..=WINDOW_BLOCKS {
        assert!(!policy.should_compress(compressible()));
    }
    assert!(policy.should_compress(compressible()));
}

#[test]
fn speed_task_lz4_policy_probes_every_eight_mebibytes() {
    assert_eq!(PROBE_BYTES, 64 * 1024);
    assert_eq!(PROBE_PERIOD, 8 * MIB);
    let mut policy = Policy::new();
    policy.start_file();
    assert!(policy.probe_due(0), "the first block of a file");
    assert!(!policy.probe_due(256 * 1024));
    assert!(!policy.probe_due(8 * MIB - 1));
    assert!(policy.probe_due(8 * MIB));
    assert!(!policy.probe_due(8 * MIB + 256 * 1024));
    assert!(policy.probe_due(16 * MIB));
    assert!(
        policy.probe_due(40 * MIB),
        "a jump over several periods samples once"
    );
    assert!(!policy.probe_due(40 * MIB + 1));
    policy.start_file();
    assert!(policy.probe_due(0), "every file starts with a sample");
}

#[test]
fn speed_task_lz4_policy_measures_itself() {
    let mut policy = Policy::new();
    let start = Instant::now();
    // Nothing measured: nothing is compressed.
    assert!(!policy.input(4.0).favours_compression());
    assert_eq!(policy.input(4.0).ratio, 1.0);

    // A text block of 256 KiB shrank to 30 % in 0.75 ms.
    policy.observe_compression(262_144, 78_643, Duration::from_micros(750));
    let ratio = policy.ratio().expect("one sample");
    assert!((ratio - 0.3).abs() < 0.001, "{ratio}");
    assert!(policy.input(4.0).core_bps > 300_000_000);

    // One second at 12.5 MB/s with the writer waiting for 900 ms.
    policy.record_send(1_000_000, Duration::from_millis(450), start);
    policy.record_send(11_500_000, Duration::from_millis(450), start + METER_WINDOW);
    let input = policy.input(4.0);
    assert!(input.writer_blocked_share > 0.5, "{input:?}");
    assert!(
        (12_000_000..=13_000_000).contains(&input.wire_bps),
        "{input:?}"
    );
    assert!(input.favours_compression());
    assert!(
        policy.decide(|| 4.0),
        "the first decision uses these figures"
    );
}
