//! The daylight-saving rule of the sync comparison (W10 of the second
//! review): when either side stores local time (FAT), equal sizes exactly
//! one hour apart are the same file only when content keys formed where each
//! file is stored show the same bytes; without them the hour counts like any
//! other time difference. No other pair gets the rule.
use super::*;
use crate::vfs::MtimePrecision;

const T: i64 = 1_700_000_000_000;
const HOUR: i64 = 3_600_000;

fn sig(size: u64, mtime_ms: i64) -> Sig {
    Sig {
        size,
        mtime_ms,
        hash: 0,
    }
}

fn hashed(size: u64, mtime_ms: i64, hash: u64) -> Sig {
    Sig {
        size,
        mtime_ms,
        hash,
    }
}

fn same(a: MtimePrecision, b: MtimePrecision, x: Sig, y: Sig) -> bool {
    let opts = BisyncOptions::default();
    let rules = TimeRules { a, b };
    same_file(x, y, rules.across(), &opts, Against::OtherSide)
}

fn apart(a: MtimePrecision, b: MtimePrecision, x: Sig, y: Sig) -> bool {
    hour_apart(x, y, TimeRules { a, b }.across(), &BisyncOptions::default())
}

#[test]
fn abgleich_task_fat_hour_without_content_keys_is_a_time_difference() {
    use MtimePrecision::*;
    for other in [Nanos, Millis, Seconds, TwoSeconds, Minutes] {
        for (x, y) in [(sig(5, T), sig(5, T + HOUR)), (sig(5, T + HOUR), sig(5, T))] {
            assert!(
                !same(TwoSeconds, other, x, y),
                "{other:?}: an hour newer is newer"
            );
            assert!(!same(other, TwoSeconds, x, y), "{other:?}");
            assert!(
                apart(TwoSeconds, other, x, y),
                "{other:?}: keys would decide"
            );
            assert!(apart(other, TwoSeconds, x, y), "{other:?}");
        }
    }
    assert!(apart(
        TwoSeconds,
        Nanos,
        sig(5, T),
        sig(5, T - HOUR - 1_500)
    ));
    assert!(!apart(
        TwoSeconds,
        Nanos,
        sig(5, T),
        sig(5, T + HOUR + 3_000)
    ));
    assert!(
        !apart(TwoSeconds, Nanos, sig(5, T), sig(6, T + HOUR)),
        "sizes differ"
    );
    let opts = BisyncOptions::default();
    assert!(
        worth_hashing(Some(sig(5, T)), Some(sig(5, T + HOUR)), &opts),
        "the planner may have both sides hashed where they are stored"
    );
}

#[test]
fn abgleich_task_fat_hour_with_content_keys_follows_the_keys() {
    use MtimePrecision::*;
    let (x, y) = (hashed(5, T, 77), hashed(5, T + HOUR, 77));
    assert!(same(TwoSeconds, Nanos, x, y), "same bytes: the clock moved");
    assert!(!apart(TwoSeconds, Nanos, x, y), "nothing left to prove");
    let changed = hashed(5, T + HOUR, 78);
    assert!(
        !same(TwoSeconds, Nanos, x, changed),
        "other bytes: a real change"
    );
    // A key on one side only proves nothing.
    assert!(!same(TwoSeconds, Nanos, x, sig(5, T + HOUR)));
    assert!(apart(TwoSeconds, Nanos, x, sig(5, T + HOUR)));
}

#[test]
fn abgleich_task_hour_shift_needs_a_local_time_side_and_the_time_mode() {
    use MtimePrecision::*;
    for (a, b) in [
        (Nanos, Nanos),
        (Seconds, Nanos),
        (TenMillis, Nanos),
        (Minutes, Seconds),
    ] {
        assert!(!same(a, b, sig(5, T), sig(5, T + HOUR)), "{a:?}/{b:?}");
        assert!(!apart(a, b, sig(5, T), sig(5, T + HOUR)), "{a:?}/{b:?}");
    }
    let sizes = BisyncOptions {
        compare: CompareMode::SizeOnly,
        ..BisyncOptions::default()
    };
    let fat = TimeRules {
        a: TwoSeconds,
        b: Nanos,
    };
    assert!(!hour_apart(
        sig(5, T),
        sig(5, T + HOUR),
        fat.across(),
        &sizes
    ));
    // Within the window there is nothing to prove.
    assert!(!apart(TwoSeconds, Nanos, sig(5, T), sig(5, T + 1_000)));
    assert!(same(TwoSeconds, Nanos, sig(5, T), sig(5, T + 1_000)));
}

/// A FAT side an hour off its own record without content keys has changed;
/// the time question alone (`same_time`) still sees the shift.
#[test]
fn abgleich_task_fat_record_an_hour_off_is_a_change_without_keys() {
    use MtimePrecision::*;
    let opts = BisyncOptions::default();
    let fat = TimeRules {
        a: TwoSeconds,
        b: Nanos,
    };
    for side in [PairSide::A, PairSide::B] {
        assert!(
            !same_file(
                sig(5, T),
                sig(5, T + HOUR),
                fat.side(side),
                &opts,
                Against::Baseline,
            ),
            "{side:?}"
        );
        assert!(same_file(
            hashed(5, T, 9),
            hashed(5, T + HOUR, 9),
            fat.side(side),
            &opts,
            Against::Baseline,
        ));
    }
    assert!(same_time(T, T + HOUR, TwoSeconds, 0));
    assert!(!same_time(T, T + HOUR, Seconds, 0));
}
