//! The decisions of the QUIC statistics of Share transfers (diagnosis of the
//! speed stage): when traffic is a burst, what a burst reports, how the kernel
//! and process counters are read, and when the file makes room.
use std::path::Path;
use std::time::Duration;

use iroh::endpoint::{ConnectionStats, PathStats};

use super::quic_stats::{
    Ambient, PathKind, Report, Sample, Tracker, BUSY_BYTES_PER_TICK, IDLE_TICKS_TO_END, TICK,
};
use super::quic_stats_hub::{needs_rotation, rotated_path, MAX_FILE_BYTES};
use super::quic_stats_proc::{
    busiest_threads, parse_stat_cpu, parse_udp_snmp, ThreadCpu, UdpCounters,
};
use super::transfer_meter::{MeterSnapshot, Phase, TransferMeter};

const MIB: u64 = 1024 * 1024;

fn tick_ms(index: u64) -> u64 {
    index * TICK.as_millis() as u64
}

/// A sample with the given totals of sent and received UDP bytes.
fn sample(tx: u64, rx: u64) -> Sample {
    Sample {
        udp_tx_bytes: tx,
        udp_rx_bytes: rx,
        udp_tx_datagrams: tx / 1200,
        udp_rx_datagrams: rx / 1200,
        rtt: Duration::from_millis(2),
        cwnd: 100_000,
        mtu: 1452,
        paths_open: 1,
        path_kind: PathKind::Ip,
        ..Sample::default()
    }
}

/// A tracker that has seen its first (quiet) tick.
fn started() -> Tracker {
    let mut tracker = Tracker::default();
    let step = tracker.observe(tick_ms(0), sample(0, 0), Ambient::default());
    assert!(!step.started && step.report.is_none());
    tracker
}

#[test]
fn speed_task_share_stats_keepalives_and_listings_are_no_burst() {
    let mut tracker = started();
    for index in 1..=20 {
        // 4 KiB per tick: keepalives, a directory listing, an ack stream.
        let step = tracker.observe(
            tick_ms(index),
            sample(index * 2048, index * 2048),
            Ambient::default(),
        );
        assert!(!step.started && step.report.is_none(), "tick {index}");
    }
    assert!(tracker.finish().is_none());
}

#[test]
fn speed_task_share_stats_threshold_is_per_tick_and_inclusive() {
    let mut tracker = started();
    let below = tracker.observe(
        tick_ms(1),
        sample(BUSY_BYTES_PER_TICK - 1, 0),
        Ambient::default(),
    );
    assert!(!below.started, "one byte below the threshold is quiet");
    // Both directions count together: the sum reaches the threshold.
    let at = tracker.observe(
        tick_ms(2),
        sample(BUSY_BYTES_PER_TICK - 1 + BUSY_BYTES_PER_TICK - 100, 100),
        Ambient::default(),
    );
    assert!(at.started, "the threshold itself is busy");
}

/// Three busy ticks of 1 MiB sent each, with a moving window and RTT, then
/// the quiet ticks that end the burst.
fn finished_burst() -> Report {
    let mut tracker = Tracker::default();
    // The tick before the burst: the process has used 1000 ms of CPU.
    let baseline = Ambient {
        cpu_ms: Some(1000),
        udp: Some(UdpCounters::default()),
        ..Ambient::default()
    };
    tracker.observe(tick_ms(0), sample(0, 0), baseline);
    let windows = [100_000u64, 300_000, 200_000];
    let rtts = [2u64, 6, 4];
    let lost = [0u64, 1, 3];
    let events = [0u64, 1, 1];
    for index in 1..=3usize {
        let mut now = sample(index as u64 * MIB, 40_000 * index as u64);
        now.cwnd = windows[index - 1];
        now.rtt = Duration::from_millis(rtts[index - 1]);
        now.lost_packets = lost[index - 1];
        now.lost_bytes = lost[index - 1] * 1200;
        now.congestion_events = events[index - 1];
        now.frames_tx.stream = index as u64 * 800;
        now.frames_tx.data_blocked = index as u64 - 1;
        now.frames_rx.acks = index as u64 * 400;
        let ambient = Ambient {
            meter: MeterSnapshot {
                streams: index as u64 * 2,
                raw_bytes: index as u64 * MIB,
                wire_bytes: index as u64 * MIB,
                lz4_ns: index as u64 * 1_000_000,
                net_wait_ns: index as u64 * 900_000_000,
                stage_write_ns: index as u64 * 5_000_000,
                ..MeterSnapshot::default()
            },
            cpu_ms: Some(1000 + index as u64 * 300),
            udp: Some(UdpCounters {
                rcvbuf_errors: index as u64 * 7,
                in_datagrams: index as u64 * 1000,
                ..UdpCounters::default()
            }),
        };
        let step = tracker.observe(tick_ms(index as u64), now, ambient);
        assert_eq!(step.started, index == 1, "tick {index}");
        assert!(step.report.is_none());
    }
    let last = sample(3 * MIB, 120_000);
    let mut report = None;
    for quiet in 0..IDLE_TICKS_TO_END {
        let step = tracker.observe(tick_ms(4 + quiet as u64), last, Ambient::default());
        assert_eq!(
            step.report.is_some(),
            quiet + 1 == IDLE_TICKS_TO_END,
            "quiet tick {quiet}"
        );
        report = report.or(step.report);
    }
    report.expect("the burst ended after the quiet ticks")
}

#[test]
fn speed_task_share_stats_burst_reports_differences_and_extremes() {
    let report = finished_burst();
    // The baseline is the tick before the first busy one: 500 ms before it.
    assert_eq!(report.busy_ticks, 3);
    assert_eq!(report.duration_ms, 3 * TICK.as_millis() as u64);
    assert_eq!(report.udp_tx_bytes, 3 * MIB);
    assert_eq!(report.udp_rx_bytes, 120_000);
    let expected = (3 * MIB) as f64 / 1.5 / 1e6;
    assert!((report.udp_tx_mb_per_s - expected).abs() < 1e-9);
    assert_eq!(report.cwnd_bytes, 200_000);
    assert_eq!(report.cwnd_bytes_min, 100_000);
    assert_eq!(report.cwnd_bytes_max, 300_000);
    assert_eq!(report.cwnd_bytes_mean, 200_000);
    assert_eq!(report.rtt_ms, 4.0);
    assert_eq!(report.rtt_ms_min, 2.0);
    assert_eq!(report.rtt_ms_max, 6.0);
    assert_eq!(report.mtu, 1452);
    assert_eq!(report.path_kind, PathKind::Ip);
    assert_eq!(report.congestion_events, 1);
    assert_eq!(report.lost_packets, 3);
    assert_eq!(report.lost_bytes, 3600);
    assert_eq!(report.frames_tx.stream, 2400);
    assert_eq!(report.frames_tx.data_blocked, 2);
    assert_eq!(report.frames_rx.acks, 1200);
    // The meter and the process counters count from the tick before the
    // burst (all zero there) to the last busy tick.
    assert_eq!(report.streams, 6);
    assert_eq!(report.meter.raw_bytes, 3 * MIB);
    assert_eq!(report.meter.lz4_ms, 3.0);
    assert_eq!(report.meter.net_wait_ms, 2700.0);
    assert_eq!(report.meter.stage_write_ms, 15.0);
    assert_eq!(report.process_cpu_ms, Some(900));
    assert_eq!(report.kernel_udp.map(|udp| udp.rcvbuf_errors), Some(21));
}

#[test]
fn speed_task_share_stats_report_names_every_diagnostic_value() {
    let value = serde_json::to_value(finished_burst()).expect("a report is JSON");
    let object = value.as_object().expect("an object");
    for name in [
        // smoothed RTT, window, congestion events, loss, datagrams, MTU
        "rtt_ms",
        "cwnd_bytes",
        "congestion_events",
        "lost_packets",
        "lost_bytes",
        "udp_tx_datagrams",
        "udp_tx_bytes",
        "udp_rx_datagrams",
        "udp_rx_bytes",
        "mtu",
        // streams used and the flow control that held them back
        "streams",
        "frames_tx",
        "frames_rx",
        // time per side: LZ4, writing the stage, waiting for the network
        "meter",
        "process_cpu_ms",
        "kernel_udp",
    ] {
        assert!(object.contains_key(name), "{name} is missing: {value}");
    }
    let meter = value["meter"].as_object().expect("the meter object");
    for name in ["lz4_ms", "stage_write_ms", "net_wait_ms", "disk_read_ms"] {
        assert!(meter.contains_key(name), "meter.{name} is missing");
    }
    assert!(value["frames_tx"]["stream_data_blocked"].is_u64());
    assert!(value["kernel_udp"]["rcvbuf_errors"].is_u64());
    assert_eq!(value["path_kind"], "ip");
}

#[test]
fn speed_task_share_stats_short_traffic_is_not_reported() {
    let mut tracker = started();
    let busy = tracker.observe(tick_ms(1), sample(MIB, 0), Ambient::default());
    assert!(busy.started);
    let mut report = None;
    for quiet in 0..IDLE_TICKS_TO_END {
        let step = tracker.observe(
            tick_ms(2 + quiet as u64),
            sample(MIB, 0),
            Ambient::default(),
        );
        report = report.or(step.report);
    }
    assert!(report.is_none(), "one busy tick is below MIN_BURST_TICKS");
}

#[test]
fn speed_task_share_stats_a_gap_below_the_idle_limit_keeps_one_burst() {
    let mut tracker = started();
    let mut total = 0;
    let mut reports = 0;
    // busy, one quiet tick, busy, busy: a retry's short pause is no end
    for (index, busy) in [true, false, true, true].into_iter().enumerate() {
        if busy {
            total += MIB;
        }
        let step = tracker.observe(
            tick_ms(index as u64 + 1),
            sample(total, 0),
            Ambient::default(),
        );
        reports += usize::from(step.report.is_some());
    }
    assert_eq!(reports, 0, "still one open burst");
    let report = tracker.finish().expect("the open burst ends with finish");
    assert_eq!(report.busy_ticks, 3);
    assert_eq!(report.udp_tx_bytes, 3 * MIB);
    assert!(tracker.finish().is_none(), "a burst ends once");
}

#[test]
fn speed_task_share_stats_second_burst_does_not_count_the_first() {
    let mut tracker = started();
    let mut total = 0;
    let mut reports = Vec::new();
    let plan = [
        true, true, false, false, false, // burst one: 2 MiB
        true, true, true, false, false, // burst two: 3 MiB
    ];
    for (index, busy) in plan.into_iter().enumerate() {
        if busy {
            total += MIB;
        }
        let step = tracker.observe(
            tick_ms(index as u64 + 1),
            sample(total, 0),
            Ambient::default(),
        );
        reports.extend(step.report);
    }
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].udp_tx_bytes, 2 * MIB);
    assert_eq!(reports[1].udp_tx_bytes, 3 * MIB);
    assert_eq!(reports[1].busy_ticks, 3);
}

#[test]
fn speed_task_share_stats_counters_that_fall_never_underflow() {
    // The data path switched: the new path's counters start below the old.
    let mut tracker = Tracker::default();
    let mut baseline = sample(0, 0);
    baseline.congestion_events = 9;
    baseline.lost_packets = 9;
    tracker.observe(tick_ms(0), baseline, Ambient::default());
    for index in 1..=2 {
        let mut now = sample(index * MIB, 0);
        now.congestion_events = 2;
        now.lost_packets = 2;
        tracker.observe(tick_ms(index), now, Ambient::default());
    }
    let report = tracker.finish().expect("two busy ticks");
    assert_eq!(report.congestion_events, 0);
    assert_eq!(report.lost_packets, 0);
    assert_eq!(report.udp_tx_bytes, 2 * MIB);
    assert_eq!(
        Sample::default().bytes_moved_since(&sample(MIB, MIB)),
        0,
        "a smaller total moved nothing"
    );
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn speed_task_share_stats_sample_reads_the_noq_counters() {
    let mut connection = ConnectionStats::default();
    connection.udp_tx.datagrams = 10;
    connection.udp_tx.bytes = 12_000;
    connection.udp_rx.datagrams = 5;
    connection.udp_rx.bytes = 600;
    connection.lost_packets = 3;
    connection.lost_bytes = 3_600;
    connection.frame_tx.stream = 8;
    connection.frame_tx.stream_data_blocked = 2;
    connection.frame_rx.acks = 4;
    let mut path = PathStats::default();
    path.rtt = Duration::from_millis(50);
    path.cwnd = 640_000;
    path.congestion_events = 2;
    path.spurious_congestion_events = 1;
    path.current_mtu = 1452;
    path.sent_plpmtud_probes = 4;
    path.lost_plpmtud_probes = 1;
    path.black_holes_detected = 1;
    let sample = Sample::from_noq(&connection, &path, PathKind::Relay, 2);
    assert_eq!(sample.udp_tx_datagrams, 10);
    assert_eq!(sample.udp_tx_bytes, 12_000);
    assert_eq!(sample.udp_rx_bytes, 600);
    assert_eq!(sample.lost_packets, 3);
    assert_eq!(sample.lost_bytes, 3_600);
    assert_eq!(sample.congestion_events, 2);
    assert_eq!(sample.spurious_congestion_events, 1);
    assert_eq!(sample.rtt, Duration::from_millis(50));
    assert_eq!(sample.cwnd, 640_000);
    assert_eq!(sample.mtu, 1452);
    assert_eq!(sample.plpmtud_probes_sent, 4);
    assert_eq!(sample.plpmtud_probes_lost, 1);
    assert_eq!(sample.black_holes, 1);
    assert_eq!(sample.frames_tx.stream, 8);
    assert_eq!(sample.frames_tx.stream_data_blocked, 2);
    assert_eq!(sample.frames_rx.acks, 4);
    assert_eq!(sample.paths_open, 2);
    assert_eq!(sample.path_kind, PathKind::Relay);
    assert_eq!(sample.bytes_moved_since(&Sample::default()), 12_600);
}

#[test]
fn speed_task_share_meter_adds_phases_and_differences_snapshots() {
    let meter = TransferMeter::default();
    let before = meter.snapshot();
    meter.stream_started();
    meter.stream_started();
    meter.bytes(1000, 400);
    meter.lz4_block();
    meter.add(Phase::Lz4, Duration::from_micros(1500));
    meter.add(Phase::NetWait, Duration::from_millis(30));
    meter.add(Phase::NetWait, Duration::from_millis(12));
    meter.add(Phase::StageWrite, Duration::from_millis(7));
    meter.add(Phase::StageWait, Duration::from_millis(1));
    meter.add(Phase::DiskRead, Duration::from_millis(2));
    meter.add(Phase::DiskWait, Duration::from_millis(3));
    let after = meter.snapshot();
    let spent = after.since(before);
    assert_eq!(spent.streams, 2);
    assert_eq!((spent.raw_bytes, spent.wire_bytes), (1000, 400));
    assert_eq!(spent.lz4_blocks, 1);
    assert_eq!(spent.lz4_ns, 1_500_000);
    assert_eq!(spent.net_wait_ns, 42_000_000);
    assert_eq!(spent.stage_write_ns, 7_000_000);
    assert_eq!(spent.stage_wait_ns, 1_000_000);
    assert_eq!(spent.disk_read_ns, 2_000_000);
    assert_eq!(spent.disk_wait_ns, 3_000_000);
    // A snapshot "before" a later one never underflows.
    assert_eq!(before.since(after), MeterSnapshot::default());
}

const SNMP: &str = "\
Ip: Forwarding DefaultTTL InReceives
Ip: 1 64 12345
Udp: InDatagrams NoPorts InErrors OutDatagrams RcvbufErrors SndbufErrors InCsumErrors IgnoredMulti MemErrors
Udp: 1000 2 3 4000 17 5 6 0 0
UdpLite: InDatagrams NoPorts
UdpLite: 0 0
";

#[test]
fn speed_task_share_stats_kernel_udp_counters_parse_by_column_name() {
    let counters = parse_udp_snmp(SNMP).expect("the Udp row");
    assert_eq!(
        counters,
        UdpCounters {
            in_datagrams: 1000,
            no_ports: 2,
            in_errors: 3,
            out_datagrams: 4000,
            rcvbuf_errors: 17,
            sndbuf_errors: 5,
            in_csum_errors: 6,
        }
    );
    // A kernel with other columns, in another order.
    let reordered = "Udp: MemErrors SndbufErrors RcvbufErrors\nUdp: 9 8 7\n";
    let counters = parse_udp_snmp(reordered).expect("a reordered row");
    assert_eq!((counters.sndbuf_errors, counters.rcvbuf_errors), (8, 7));
    assert_eq!(counters.in_datagrams, 0);
    assert!(parse_udp_snmp("Ip: A B\nIp: 1 2\n").is_none());
    assert!(parse_udp_snmp("Udp: InDatagrams\n").is_none());
    let before = UdpCounters {
        rcvbuf_errors: 10,
        ..UdpCounters::default()
    };
    let after = UdpCounters {
        rcvbuf_errors: 25,
        ..UdpCounters::default()
    };
    assert_eq!(after.since(before).rcvbuf_errors, 15);
    assert_eq!(before.since(after).rcvbuf_errors, 0);
}

#[test]
fn speed_task_share_stats_cpu_time_reads_names_with_spaces_and_brackets() {
    let plain = "1234 (share-iroh) S 1 1 1 0 -1 4194304 10 0 0 0 250 125 0 0 20 0 8 0 100 0 0";
    assert_eq!(
        parse_stat_cpu(plain),
        Some(("share-iroh".to_string(), 3750))
    );
    let tricky = "42 (a (b) c) R 1 1 1 0 -1 0 0 0 0 0 7 3 0 0 20 0 1 0 5";
    assert_eq!(parse_stat_cpu(tricky), Some(("a (b) c".to_string(), 100)));
    assert_eq!(parse_stat_cpu("42 (short) R 1 2"), None);
    assert_eq!(parse_stat_cpu("no brackets here"), None);
    assert_eq!(parse_stat_cpu("1 (x) S a b c d e f g h i j k l m n"), None);
}

fn thread(tid: u64, name: &str, cpu_ms: u64) -> ThreadCpu {
    ThreadCpu {
        tid,
        name: name.to_string(),
        cpu_ms,
    }
}

#[test]
fn speed_task_share_stats_busiest_threads_are_ranked_by_cpu_since_the_start() {
    let before = [thread(1, "main", 500), thread(2, "share-iroh", 1000)];
    let after = [
        thread(1, "main", 500),
        thread(2, "share-iroh", 1900),
        thread(3, "share-iroh", 400),
        thread(4, "blocking", 400),
        thread(5, "idle", 50),
    ];
    let busiest = busiest_threads(&before, &after, 3);
    let seen: Vec<(u64, u64)> = busiest.iter().map(|t| (t.tid, t.cpu_ms)).collect();
    // Thread 1 used nothing; a thread that began later counts from zero;
    // equal times are ordered by thread id.
    assert_eq!(seen, vec![(2, 900), (3, 400), (4, 400)]);
    assert!(busiest_threads(&after, &before, 3).is_empty());
}

#[test]
fn speed_task_share_stats_file_makes_room_before_it_exceeds_the_limit() {
    assert!(!needs_rotation(0, 2_000));
    assert!(!needs_rotation(MAX_FILE_BYTES - 10, 10));
    assert!(needs_rotation(MAX_FILE_BYTES - 9, 10));
    assert!(needs_rotation(u64::MAX, 1), "no overflow at the extreme");
    assert_eq!(
        rotated_path(Path::new("/data/smart_explorer/share-quic-stats.jsonl")),
        Path::new("/data/smart_explorer/share-quic-stats.jsonl.1")
    );
}
