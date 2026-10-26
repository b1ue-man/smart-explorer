//! The production presentation ledger accepts only a sent current identity.
use super::*;

#[test]
fn batches_task_b6_presentation_requires_sent_current_gop() {
    let mut reference = ReferencePresentation::new();
    assert!(!reference.presented(7, 1));
    reference.sent(7, 1, false);
    assert!(!reference.presented(7, 1));
    reference.sent(7, 1, true);
    assert!(!reference.ready);
    assert!(!reference.presented(8, 1));
    assert!(!reference.presented(7, 2));
    assert!(!reference.ready);
    assert!(reference.presented(7, 1));
    assert!(reference.ready);

    reference.sent(8, 2, true);
    assert!(!reference.ready);
    assert!(!reference.presented(7, 1));
    assert!(reference.presented(8, 2));
}

#[test]
fn batches_task_b6_presentation_invalidation_requires_new_reference() {
    let mut reference = ReferencePresentation::new();
    reference.sent(7, 10, true);
    assert!(reference.presented(7, 10));
    reference.invalidate(11);
    assert!(!reference.ready);
    assert_eq!(reference.floor, 11);
    assert!(!reference.presented(7, 10));
    reference.sent(8, 10, true);
    assert!(!reference.presented(8, 10));
    reference.sent(9, 11, false);
    assert!(!reference.presented(9, 11));
    reference.sent(9, 11, true);
    assert!(!reference.ready);
    assert!(reference.presented(9, 11));
    assert!(reference.ready);
}

#[test]
fn batches_task_b6_presentation_history_evicts_old_receipts() {
    let mut reference = ReferencePresentation::new();
    reference.sent(3, 1, true);
    for frame in 2..=PRESENTATION_HISTORY as u64 + 1 {
        reference.sent(3, frame, false);
    }
    assert_eq!(reference.sent.len(), PRESENTATION_HISTORY);
    assert!(!reference.presented(3, 1));
    assert!(!reference.ready);
    assert!(reference.presented(3, PRESENTATION_HISTORY as u64 + 1));
    reference.invalidate(PRESENTATION_HISTORY as u64 + 2);
    assert!(reference.sent.is_empty());
    assert!(!reference.ready);
    assert!(!reference.presented(3, PRESENTATION_HISTORY as u64 + 1));
}
