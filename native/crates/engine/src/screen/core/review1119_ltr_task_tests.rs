//! Review 2026-11-19, Gemini round 8 finding 3.1: the long-term references
//! keep a picture the viewer confirmed (`LtrSlots::confirm`).
use super::{LtrPlan, LtrSlots, RefHistory};

/// A slot waits for the acknowledgement of its picture before it is marked
/// anew; the newest confirmed one is never overwritten; a reset drops the
/// lost pictures for good.
#[test]
fn review1119_task_ltr_keeps_a_confirmed_reference() {
    let mark = |slot| LtrPlan {
        mark: Some(slot),
        force: None,
    };
    let mut ltr = LtrSlots::new(60);
    ltr.confirm(1);
    assert_eq!(ltr.plan(100, true), mark(0));
    for frame in 101..115 {
        assert_eq!(ltr.plan(frame, false), LtrPlan::default(), "{frame}");
    }
    assert_eq!(ltr.plan(115, false), mark(1));
    for frame in 116..130 {
        ltr.plan(frame, false);
    }
    // Both pictures still wait for their acknowledgement.
    assert_eq!(ltr.plan(130, false), LtrPlan::default());
    ltr.confirm(100);
    assert_eq!(ltr.plan(131, false), LtrPlan::default(), "100 is kept");
    ltr.confirm(115);
    assert_eq!(ltr.plan(132, false), mark(0), "115 is kept, 100 goes");
    // A reset after 120: 132 was lost, 115 is the reference.
    assert!(ltr.recover(120));
    assert_eq!(ltr.plan(133, false).force, Some(1 << 1));
    // A later acknowledgement never revives the lost picture.
    ltr.confirm(140);
    ltr.drop_after(140);
    assert!(ltr.recover(140));
    assert_eq!(ltr.plan(141, false).force, Some(1 << 1));
    // An IDR of the encoder's own drops every long-term reference.
    ltr.key_frame();
    assert!(!ltr.recover(140));
    // Media Foundation reports each mark on the output, its own included.
    assert!(ltr.marked(1, 150));
    assert!(!ltr.marked(2, 151), "two slots only");
    assert!(ltr.recover(150));
    assert_eq!(ltr.plan(152, false).force, Some(1 << 1));
    // Round 10: a planned mark the encoder did not take is no reference.
    ltr.unmark(1);
    assert!(!ltr.recover(151), "152 is newer, 150 was not taken");
    let mut history = RefHistory::new(3);
    for frame in 1..=4 {
        history.encoded(frame, frame == 1);
    }
    assert_eq!(history.drain(), vec![2, 3, 4]);
    assert_eq!(history.invalidate_after(4), None);
}
