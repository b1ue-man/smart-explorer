use super::*;
use crate::vfs::MtimePrecision;

fn stamp(size: u64, mtime_ms: i64) -> Stamp {
    Stamp { size, mtime_ms }
}

#[test]
fn abgleich_task_rule_equal_when_size_equal_and_times_within_window() {
    assert_eq!(
        classify(stamp(10, 1_000_000), stamp(10, 1_000_000), 1),
        Case::Equal
    );
    assert_eq!(
        classify(stamp(10, 1_002_000), stamp(10, 1_000_000), 2_000),
        Case::Equal,
        "a distance of exactly the window is equal"
    );
    assert_eq!(
        classify(stamp(10, 1_000_000), stamp(10, 1_002_000), 2_000),
        Case::Equal
    );
}

#[test]
fn abgleich_task_rule_source_newer_replaces_and_target_newer_stays() {
    assert_eq!(
        classify(stamp(10, 1_002_001), stamp(10, 1_000_000), 2_000),
        Case::SourceNewer
    );
    assert_eq!(
        classify(stamp(99, 5_000_000), stamp(10, 1_000_000), 2_000),
        Case::SourceNewer,
        "a newer source wins whatever the sizes are"
    );
    assert_eq!(
        classify(stamp(10, 1_000_000), stamp(10, 1_002_001), 2_000),
        Case::TargetNewer
    );
    assert_eq!(
        classify(stamp(1, 1_000_000), stamp(99, 9_000_000), 2_000),
        Case::TargetNewer,
        "a newer target stays whatever the sizes are"
    );
}

#[test]
fn abgleich_task_rule_same_time_other_size_is_a_conflict() {
    assert_eq!(
        classify(stamp(10, 1_000_000), stamp(11, 1_000_000), 1),
        Case::Conflict
    );
    assert_eq!(
        classify(stamp(10, 1_000_000), stamp(11, 1_001_999), 2_000),
        Case::Conflict
    );
}

#[test]
fn abgleich_task_rule_unknown_time_is_a_conflict_whatever_the_size() {
    // Equal sizes do not prove equal contents without a time: nothing is
    // skipped (a changed source of the same size must still arrive).
    assert_eq!(
        classify(stamp(7, 0), stamp(7, 123_000), 2_000),
        Case::Conflict
    );
    assert_eq!(
        classify(stamp(7, 123_000), stamp(7, 0), 2_000),
        Case::Conflict
    );
    assert_eq!(classify(stamp(7, 0), stamp(7, 0), 2_000), Case::Conflict);
    assert_eq!(
        classify(stamp(7, 0), stamp(8, 123_000), 2_000),
        Case::Conflict
    );
}

#[test]
fn abgleich_task_replacing_holds_only_for_the_target_that_was_decided_against() {
    let replacing = Replacing {
        id: None,
        stamp: stamp(10, 1_000_000),
        window_ms: 2_000,
    };
    assert!(replacing.holds(stamp(10, 1_000_000)));
    assert!(replacing.holds(stamp(10, 1_002_000)), "within the window");
    assert!(
        !replacing.holds(stamp(10, 1_002_001)),
        "saved again meanwhile: newer than what the decision saw"
    );
    assert!(!replacing.holds(stamp(11, 1_000_000)), "another size");
    assert!(
        !replacing.holds(stamp(10, 0)),
        "an unknown time proves nothing"
    );
}

#[test]
fn abgleich_task_rule_extreme_times_do_not_overflow() {
    assert_eq!(
        classify(stamp(1, i64::MAX), stamp(1, i64::MIN + 1), 2_000),
        Case::SourceNewer
    );
    assert_eq!(
        classify(stamp(1, i64::MIN + 1), stamp(1, i64::MAX), 2_000),
        Case::TargetNewer
    );
}

#[test]
fn abgleich_task_window_is_the_coarser_resolution_unknown_two_seconds() {
    use MtimePrecision::*;
    assert_eq!(window_ms(Nanos, Nanos), 1);
    assert_eq!(window_ms(Nanos, Millis), 1);
    assert_eq!(window_ms(Nanos, TenMillis), 10);
    assert_eq!(window_ms(Nanos, Seconds), 1_000);
    assert_eq!(window_ms(TwoSeconds, Nanos), 2_000);
    assert_eq!(window_ms(Nanos, Minutes), 60_000);
    assert_eq!(window_ms(Days, Seconds), 86_400_000);
    assert_eq!(window_ms(Unknown, Unknown), UNKNOWN_WINDOW_MS);
    assert_eq!(window_ms(Nanos, Unknown), UNKNOWN_WINDOW_MS);
    assert_eq!(window_ms(Unknown, Minutes), 60_000);
}

#[test]
fn abgleich_task_summary_names_counts_only() {
    let mut progress = TransferProgress::new(crate::transfer::TransferKind::Local, "x", 0, 0);
    assert_eq!(summary(&progress), None);
    progress.replaced = 4;
    progress.skipped_equal = 40;
    progress.kept_newer = 2;
    progress.kept_both = 1;
    assert_eq!(
        summary(&progress).as_deref(),
        Some("4 neuere ersetzt, 40 gleiche übersprungen, 2 Ziel neuer behalten, 1 als Konflikt doppelt behalten")
    );
}
