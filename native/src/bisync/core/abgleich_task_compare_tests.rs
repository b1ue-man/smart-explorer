//! The daylight-saving rule of the sync comparison (E8, plan.md E): when
//! either side stores local time (FAT), equal sizes exactly one hour apart
//! (within one step of the common precision) are the same file, whatever
//! the other side's precision; no other pair gets the rule.
use super::*;
use crate::vfs::MtimePrecision;

const T: i64 = 1_700_000_000_000;

fn sig(size: u64, mtime_ms: i64) -> Sig {
    Sig {
        size,
        mtime_ms,
        hash: 0,
    }
}

fn same(a: MtimePrecision, b: MtimePrecision, x: Sig, y: Sig) -> bool {
    let opts = BisyncOptions::default();
    let rules = TimeRules { a, b };
    same_file(x, y, rules.across(), &opts, Against::OtherSide)
}

#[test]
fn abgleich_task_fat_side_takes_an_hour_shift_as_equal() {
    use MtimePrecision::*;
    for other in [Nanos, Millis, Seconds, TwoSeconds, Minutes] {
        assert!(
            same(TwoSeconds, other, sig(5, T), sig(5, T + 3_600_000)),
            "{other:?}"
        );
        assert!(
            same(other, TwoSeconds, sig(5, T + 3_600_000), sig(5, T)),
            "{other:?}"
        );
    }
    assert!(same(TwoSeconds, Nanos, sig(5, T), sig(5, T - 3_601_500)));
    assert!(!same(TwoSeconds, Nanos, sig(5, T), sig(5, T + 3_603_000)));
    assert!(
        !same(TwoSeconds, Nanos, sig(5, T), sig(6, T + 3_600_000)),
        "sizes differ"
    );
}

#[test]
fn abgleich_task_hour_shift_needs_a_local_time_side() {
    use MtimePrecision::*;
    for (a, b) in [
        (Nanos, Nanos),
        (Seconds, Nanos),
        (TenMillis, Nanos),
        (Minutes, Seconds),
    ] {
        assert!(!same(a, b, sig(5, T), sig(5, T + 3_600_000)), "{a:?}/{b:?}");
    }
    // The one-side rule of a record still follows that side's precision.
    let opts = BisyncOptions::default();
    let fat = TimeRules {
        a: TwoSeconds,
        b: Nanos,
    };
    for (side, expected) in [(PairSide::A, true), (PairSide::B, false)] {
        let shifted = same_file(
            sig(5, T),
            sig(5, T + 3_600_000),
            fat.side(side),
            &opts,
            Against::Baseline,
        );
        assert_eq!(shifted, expected, "{side:?}");
    }
}
