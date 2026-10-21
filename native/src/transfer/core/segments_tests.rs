//! Parts of a large file: split threshold, part lengths, the ledger of which
//! parts are done and the safe start a retry continues from.
use super::segments::{splits, Ledger, SegmentSizer, MIN_SEGMENT, SPLIT_FROM};

const MIB: u64 = 1024 * 1024;

#[test]
fn speed_task_segments_split_threshold_is_two_min_segments() {
    assert_eq!(MIN_SEGMENT, 8 * MIB);
    assert_eq!(SPLIT_FROM, 16 * MIB);
    assert!(!splits(0));
    assert!(!splits(16 * MIB - 1), "one byte below two shortest parts");
    assert!(splits(16 * MIB));
    assert!(splits(40 * 1024 * MIB));
}

#[test]
fn speed_task_segments_ledger_claims_in_order() {
    let sizer = SegmentSizer::default();
    let mut ledger = Ledger::new(40 * MIB, 0);
    let mut claimed = Vec::new();
    while let Some(part) = ledger.claim(&sizer) {
        claimed.push(part);
    }
    assert_eq!(
        claimed,
        vec![
            (0, 8 * MIB),
            (8 * MIB, 8 * MIB),
            (16 * MIB, 8 * MIB),
            (24 * MIB, 8 * MIB),
            (32 * MIB, 8 * MIB),
        ]
    );
    assert!(!ledger.has_unclaimed());
    assert!(!ledger.all_done(), "five parts still run");
    for (start, _) in claimed {
        ledger.complete(start);
    }
    assert!(ledger.all_done());
}

#[test]
fn speed_task_segments_prefix_is_smallest_inflight_start() {
    let sizer = SegmentSizer::default();
    let mut ledger = Ledger::new(40 * MIB, 0);
    for _ in 0..3 {
        ledger.claim(&sizer);
    }
    assert_eq!(ledger.prefix(), 0, "the first part still runs");
    ledger.complete(8 * MIB);
    assert_eq!(ledger.prefix(), 0, "a finished later part does not move it");
    ledger.complete(0);
    assert_eq!(ledger.prefix(), 16 * MIB, "the smallest part still running");
    ledger.complete(16 * MIB);
    assert_eq!(
        ledger.prefix(),
        24 * MIB,
        "nothing runs: the next unclaimed byte"
    );
}

#[test]
fn speed_task_segments_prefix_stops_at_failed_range() {
    let sizer = SegmentSizer::default();
    let mut ledger = Ledger::new(40 * MIB, 0);
    for _ in 0..3 {
        ledger.claim(&sizer);
    }
    ledger.complete(16 * MIB);
    ledger.release_failed(8 * MIB);
    ledger.complete(0);
    assert!(ledger.in_flight_empty());
    assert_eq!(
        ledger.prefix(),
        8 * MIB,
        "bytes after a failed part are not safe"
    );
    assert!(!ledger.all_done());
    assert!(ledger.has_unclaimed());
    ledger.release_failed(16 * MIB);
    assert_eq!(ledger.prefix(), 8 * MIB, "the smallest failed start stays");
}

#[test]
fn speed_task_segments_resume_at_prefix_skips_done_bytes() {
    let sizer = SegmentSizer::default();
    let mut ledger = Ledger::new(40 * MIB, 16 * MIB);
    assert_eq!(ledger.prefix(), 16 * MIB);
    assert_eq!(ledger.claim(&sizer), Some((16 * MIB, 8 * MIB)));
    let mut finished = Ledger::new(40 * MIB, 40 * MIB);
    assert!(finished.claim(&sizer).is_none());
    assert!(finished.all_done(), "everything was written before");
    let beyond = Ledger::new(40 * MIB, 99 * MIB);
    assert_eq!(
        beyond.prefix(),
        40 * MIB,
        "a resume point is never past the end"
    );
}

#[test]
fn speed_task_segments_last_range_takes_remainder_below_min() {
    let sizer = SegmentSizer::default();
    let mut ledger = Ledger::new(20 * MIB, 0);
    assert_eq!(ledger.claim(&sizer), Some((0, 8 * MIB)));
    assert_eq!(
        ledger.claim(&sizer),
        Some((8 * MIB, 12 * MIB)),
        "4 MiB would be left: the part takes them"
    );
    assert!(ledger.claim(&sizer).is_none());
    let mut odd = Ledger::new(16 * MIB + 1, 0);
    assert_eq!(odd.claim(&sizer), Some((0, 8 * MIB)));
    assert_eq!(odd.claim(&sizer), Some((8 * MIB, 8 * MIB + 1)));
    assert_eq!(
        SegmentSizer::default().next_len(3 * MIB),
        3 * MIB,
        "never above the rest"
    );
}

#[test]
fn speed_task_segments_sizer_defaults_to_min_then_follows_rate_and_setup() {
    let mut sizer = SegmentSizer::default();
    assert_eq!(
        sizer.next_len(1024 * MIB),
        MIN_SEGMENT,
        "no measurement yet"
    );
    sizer.record(0, 100, 200);
    sizer.record(8 * MIB, 500, 500);
    assert_eq!(
        sizer.next_len(1024 * MIB),
        MIN_SEGMENT,
        "unusable measurements are ignored"
    );
    // 8 MiB/s after a set-up of 0.5 s: ten set-ups of stream.
    sizer.record(8 * MIB, 500, 1500);
    assert_eq!(sizer.next_len(1024 * MIB), 40 * MIB);
    // A second part at 16 MiB/s moves the mean by the weight 0.3.
    sizer.record(16 * MIB, 500, 1500);
    let blended = sizer.next_len(1024 * MIB);
    assert!(
        blended > 51 * MIB && blended < 53 * MIB,
        "10 x 0.5 s x 10.4 MiB/s, got {blended}"
    );
    // A slow link with quick set-up never goes below the shortest part.
    let mut slow = SegmentSizer::default();
    slow.record(8 * MIB, 50, 8050);
    assert_eq!(slow.next_len(1024 * MIB), MIN_SEGMENT);
}

#[test]
fn speed_task_segments_sizer_caps_at_thirty_seconds_of_stream_rate() {
    let mut sizer = SegmentSizer::default();
    // 1 GiB/s after 10 s of set-up would ask for 100 GiB.
    sizer.record(1024 * MIB, 10_000, 11_000);
    assert_eq!(sizer.next_len(100 * 1024 * MIB), 30 * 1024 * MIB);
    assert_eq!(
        sizer.next_len(30 * 1024 * MIB + 4 * MIB),
        30 * 1024 * MIB + 4 * MIB,
        "the rest below the shortest part joins the last one"
    );
}
