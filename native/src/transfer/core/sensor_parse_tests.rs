//! The sensor parsers read kernel text; a missing or garbled source is
//! "unknown", never an error.
use super::sensor_parse::*;

const STAT_A: &str = "cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 50 0 50 350 50 0 0 0 0 0\n";
const STAT_B: &str = "cpu  200 0 200 800 100 0 0 0 0 0\ncpu0 100 0 100 400 50 0 0 0 0 0\n";

#[test]
fn speed_task_sensor_parse_proc_stat_deltas() {
    let before = parse_proc_stat(STAT_A).expect("first reading");
    let after = parse_proc_stat(STAT_B).expect("second reading");
    // idle and iowait are not busy time
    assert_eq!(
        before,
        CpuTicks {
            busy: 200,
            total: 1000
        }
    );
    assert_eq!(
        after,
        CpuTicks {
            busy: 400,
            total: 1300
        }
    );
    let busy = busy_fraction(before, after).expect("time passed");
    assert!(
        (busy - 2.0 / 3.0).abs() < 1e-9,
        "200 of the 300 new ticks were busy"
    );
    assert_eq!(busy_fraction(after, after), None, "no ticks passed");
    // a counter that went backwards (reset) gives no value instead of nonsense
    assert_eq!(busy_fraction(after, before), None);
}

#[test]
fn speed_task_sensor_parse_meminfo_available() {
    let text = "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:    8192000 kB\n";
    assert_eq!(parse_meminfo_available(text), Some(8_192_000 * 1024));
    assert_eq!(parse_meminfo_available("MemTotal: 5 kB\n"), None);
    assert_eq!(parse_meminfo_available("MemAvailable: lots kB\n"), None);
}

#[test]
fn speed_task_sensor_parse_cpu_max_quota_and_affinity() {
    assert_eq!(parse_cpu_max("max 100000\n", 8), 8.0);
    assert_eq!(parse_cpu_max("200000 100000\n", 8), 2.0);
    // the quota never grants more than the affinity does
    assert_eq!(parse_cpu_max("800000 100000\n", 4), 4.0);
    assert_eq!(parse_cpu_max("50000 100000\n", 4), 0.5);
    assert_eq!(parse_cpu_max("", 4), 4.0, "unreadable keeps the affinity");
    assert_eq!(parse_cpu_max("nonsense", 4), 4.0);
    assert_eq!(parse_cpu_max("max 100000", 0), 1.0, "at least one core");
}

#[test]
fn speed_task_sensor_parse_pids_headroom() {
    assert_eq!(parse_pids("4915\n", "120\n"), Some(4795));
    assert_eq!(
        parse_pids("max\n", "120\n"),
        None,
        "no limit is no headroom"
    );
    assert_eq!(parse_pids("100", "150"), Some(0));
    assert_eq!(parse_pids("", "1"), None);
    assert_eq!(parse_process_ticks(SELF_STAT), Some(150 + 25));
}

/// A command name with spaces and a bracket must not shift the fields.
const SELF_STAT: &str =
    "4242 (se (copy) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 25 0 0 20 0 9 0 12345 1 2";

const DISKSTATS: &str = "\
   8       0 sda 100 0 800 50 100 0 800 50 0 400 1000 0 0 0 0 0 0\n\
   8       1 sda1 90 0 700 40 90 0 700 40 0 380 900 0 0 0 0 0 0\n\
 259       0 nvme0n1 5 0 40 1 5 0 40 1 0 10 20 0 0 0 0 0 0\n";

#[test]
fn speed_task_sensor_parse_diskstats_queue_depth() {
    let by_name = parse_diskstats(DISKSTATS, "sda1").expect("sda1 listed");
    assert_eq!(
        by_name,
        DiskTicks {
            io_ms: 380,
            weighted_ms: 900
        }
    );
    assert_eq!(parse_diskstats(DISKSTATS, "8:1"), Some(by_name));
    let (name, _) = find_disk(DISKSTATS, "259:0").expect("by id");
    assert_eq!(name, "nvme0n1");
    assert_eq!(parse_diskstats(DISKSTATS, "sdz"), None);
    let later = DiskTicks {
        io_ms: 980,
        weighted_ms: 3_900,
    };
    // 3 000 weighted ms in 1 000 ms of wall time: three requests in flight
    assert!((queue_depth(by_name, later, 1_000) - 3.0).abs() < 1e-9);
    assert_eq!(queue_depth(by_name, later, 0), 0.0);
    assert_eq!(queue_depth(later, by_name, 1_000), 0.0, "counter reset");
}

#[test]
fn speed_task_sensor_parse_route_and_link_speed() {
    let route = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                 eth1\t00000000\t0102A8C0\t0003\t0\t0\t600\t00000000\n\
                 eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
                 eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
    assert_eq!(parse_default_route(route).as_deref(), Some("eth0"));
    assert_eq!(parse_default_route("Iface\tDestination\n"), None);
    assert_eq!(parse_link_speed("1000\n"), Some(125_000_000));
    assert_eq!(parse_link_speed("-1\n"), None, "unknown link speed");
    assert_eq!(parse_link_speed("Invalid argument"), None);
}

#[test]
fn speed_task_sensor_unreadable_source_gives_none() {
    assert_eq!(parse_proc_stat(""), None);
    assert_eq!(parse_proc_stat("intr 1 2 3\n"), None);
    assert_eq!(parse_proc_stat("cpu  a b c d\n"), None);
    assert_eq!(parse_proc_stat("cpu  1 2 3\n"), None, "too few fields");
    assert_eq!(parse_meminfo_available(""), None);
    assert_eq!(parse_pids("", ""), None);
    assert_eq!(parse_process_ticks(""), None);
    assert_eq!(parse_process_ticks("1 (x) S 1"), None);
    assert_eq!(parse_diskstats("", "sda"), None);
    assert_eq!(parse_diskstats("8 0 sda 1 2", "sda"), None, "short row");
    assert_eq!(parse_link_speed(""), None);
    // a reading where nothing could be read gives a sample without values
    let nothing = RawSample::default();
    let sample = sample_from(Some(&nothing), &nothing, 1_000);
    assert_eq!(sample.cpu_busy, None);
    assert_eq!(sample.mem_available, None);
    assert_eq!(sample.threads_free, None);
    assert_eq!(sample.link_bps, None);
    assert_eq!(sample.disk_queue, None);
    assert!(!sample.process_cpu_known);
    assert_eq!(sample.free_cores(), None);
    assert!(
        sample.cores_available >= 1.0,
        "falls back to the parallelism"
    );
}

#[test]
fn speed_task_sensor_sample_from_two_readings() {
    let ticks = |ns: u64, weighted: u64| RawSample {
        cpu: parse_proc_stat(STAT_A),
        process_ns: Some(ns),
        cores: Some(4.0),
        mem_available: Some(1 << 30),
        threads_free: Some(100),
        link_bps: Some(125_000_000),
        disks: vec![(
            "8:1".to_string(),
            DiskTicks {
                io_ms: 0,
                weighted_ms: weighted,
            },
        )],
    };
    let first = ticks(1_000_000_000, 100);
    let second = ticks(3_000_000_000, 2_100);
    let alone = sample_from(None, &first, 0);
    assert_eq!(alone.mem_available, Some(1 << 30));
    assert!(!alone.process_cpu_known, "needs two readings");
    let sample = sample_from(Some(&first), &second, 1_000);
    // 2 s of CPU in 1 s of wall time: two cores busy
    assert!((sample.process_cpu_cores - 2.0).abs() < 1e-9);
    assert_eq!(sample.free_cores(), Some(2.0));
    assert_eq!(sample.disk_queue, Some(2.0));
    assert_eq!(sample.disk_queues, vec![("8:1".to_string(), 2.0)]);
}
