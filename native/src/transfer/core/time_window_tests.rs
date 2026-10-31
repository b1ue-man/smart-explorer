use super::*;

#[test]
fn abgleich_task_dst_shift_is_exactly_one_hour_within_the_window() {
    let base = 1_700_000_000_000_i64;
    assert!(dst_shift_equal(base, base + HOUR_MS, 0));
    assert!(dst_shift_equal(base + HOUR_MS, base, 0), "order is free");
    assert!(dst_shift_equal(base, base + HOUR_MS + 2_000, 2_000));
    assert!(dst_shift_equal(base, base + HOUR_MS - 2_000, 2_000));
    assert!(!dst_shift_equal(base, base + HOUR_MS + 2_001, 2_000));
    assert!(!dst_shift_equal(base, base + HOUR_MS - 2_001, 2_000));
}

#[test]
fn abgleich_task_dst_shift_is_no_shift_for_equal_or_far_times() {
    let base = 1_700_000_000_000_i64;
    assert!(!dst_shift_equal(base, base, 2_000));
    assert!(!dst_shift_equal(base, base + 2 * HOUR_MS, 2_000));
    assert!(!dst_shift_equal(base, base + 30 * 60_000, 2_000));
}

#[test]
fn abgleich_task_dst_shift_never_overflows() {
    assert!(!dst_shift_equal(i64::MIN, i64::MAX, 2_000));
    assert!(!dst_shift_equal(i64::MAX, i64::MIN, i64::MAX));
    assert!(dst_shift_equal(0, HOUR_MS, -5), "a negative window is zero");
    assert!(!dst_shift_equal(0, HOUR_MS + 1, -5));
}
