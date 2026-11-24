use std::time::Duration;

use super::SmallLane;
use crate::remote_window::OperationStatus;
use crate::screen::core::family_protocol::FamilyOperationResult;
use crate::screen::core::tiles_rate::SharedBudget;
use crate::screen::core::wire_control::{HostFrame, Pong};

/// Codex 11/7: small control frames take the share the pictures leave the
/// control stream (round 12, H2: one second of burst, the window the share
/// is measured over); only what goes beyond it is booked in the pictures'
/// budget, and the share refills at its rate.
#[test]
fn review1119_task_small_frames_take_their_share() {
    let mut lane = SmallLane::default();
    // 80 kbit/s reserved: 10 000 bytes of burst.
    assert_eq!(lane.excess(0, 80_000, 9_500), 0);
    assert_eq!(lane.excess(0, 80_000, 1_000), 500);
    // 100 ms later 1000 bytes came back.
    assert_eq!(lane.excess(100_000, 80_000, 1_000), 0);
    assert_eq!(lane.excess(100_000, 80_000, 10), 10);
    // Without a reserved share every byte takes the budget.
    let mut none = SmallLane::default();
    assert_eq!(none.excess(0, 0, 40), 40);
}

/// Round 12 (H2): a frame sent once a second (`STATS`, `PONG`) whose bytes
/// make the measured control rate fits its share every time (with 200 ms of
/// burst four fifths of it took the pictures' budget for good); a look at
/// the share (`peek`, a frame that may wait) takes nothing.
#[test]
fn r12_task_periodic_frames_fit_their_share() {
    let mut lane = SmallLane::default();
    // 600 bytes a second measure 4800 bit/s.
    for second in 0..5u64 {
        assert_eq!(lane.excess(second * 1_000_000, 4_800, 600), 0, "{second}");
    }
    let mut lane = SmallLane::default();
    assert_eq!(lane.excess(0, 8_000, 1_000), 0);
    assert_eq!(lane.peek(0, 8_000, 500), 500);
    assert_eq!(lane.peek(0, 8_000, 500), 500, "a look takes nothing");
    assert_eq!(lane.excess(500_000, 8_000, 500), 0);
}

/// Round 13 (H1, H2): the session wakes for a small frame as soon as its
/// share covers it whole (`peek` is 0 then), not only when the budget has
/// credit; a frame the share never covers (no share, more than its burst)
/// has no such time.
#[test]
fn r13_task_share_names_when_it_covers_the_head() {
    let mut lane = SmallLane::default();
    // 80 kbit/s: 10 000 bytes of burst, full before the first frame.
    assert_eq!(lane.wait_us(0, 80_000, 10_000), Some(0));
    assert_eq!(lane.excess(0, 80_000, 9_000), 0);
    // 1 000 bytes left; 2 000 need 1 000 more bytes: 100 ms at 10 000 bytes/s.
    assert_eq!(lane.wait_us(0, 80_000, 2_000), Some(100_000));
    assert!(lane.peek(99_999, 80_000, 2_000) > 0);
    assert_eq!(lane.peek(100_000, 80_000, 2_000), 0);
    assert_eq!(lane.wait_us(100_000, 80_000, 2_000), Some(0));
    assert_eq!(lane.wait_us(0, 80_000, 10_001), None, "beyond the burst");
    assert_eq!(lane.wait_us(0, 0, 1), None, "no share");
}

fn outbox(rate: Option<u64>) -> (super::Outbox, SharedBudget) {
    let budget = SharedBudget::default();
    budget.set_rate(rate);
    (super::Outbox::new(budget.clone(), rate), budget)
}

fn pong() -> HostFrame {
    HostFrame::Pong(Pong {
        sequence: 1,
        ping_sent_us: 0,
        received_us: 0,
        sent_us: 0,
    })
}

fn family_result() -> HostFrame {
    HostFrame::FamilyOperation(FamilyOperationResult {
        request_id: "r1".into(),
        status: Some(OperationStatus::Requested),
        reason: None,
        snapshot: None,
    })
}

fn writing() -> super::Writing {
    super::Writing {
        bytes: vec![0; 1 << 20],
        offset: 0,
        part_tag: Some(1),
        metadata: false,
        shape: None,
    }
}

/// Round 13 (H1): a viewer that takes parts gets small frames between the
/// parts of a large frame being written (before, they waited for its end:
/// about 8 s for a family state of 1 MiB at 1 Mbit/s); a family frame keeps
/// its place behind it, and a viewer without parts gets them after it.
#[test]
fn r13_task_small_frames_are_due_between_parts() {
    let (mut outbox, _) = outbox(Some(1_000_000));
    outbox.writing = Some(writing());
    outbox.behind.push_back(pong());
    assert!(!outbox.small_due(), "no parts: the frame is one write");
    outbox.parts = true;
    assert!(outbox.small_due());
    outbox.behind.push_front(family_result());
    assert!(
        !outbox.small_due(),
        "a family frame waits for the large one"
    );
    outbox.writing = None;
    assert!(outbox.small_due());
}

/// Round 13 (H1): with a large frame being written and the budget in debt
/// for a long time, `ready` returns at once when the share covers the small
/// frame at the head (before, it slept until the budget had credit).
#[tokio::test]
async fn r13_task_ready_wakes_for_a_small_frame_between_parts() {
    let (mut outbox, budget) = outbox(Some(1));
    // 1 000 bytes at 1 bit/s: 8 000 s of debt.
    budget.on_sent(crate::screen::core::gop_stream::now_us(), 1_000, 1);
    budget.set_reserved(80_000);
    outbox.parts = true;
    outbox.writing = Some(writing());
    outbox.behind.push_back(pong());
    let ready = tokio::time::timeout(Duration::from_secs(2), outbox.ready()).await;
    assert!(ready.is_ok(), "the share covers the frame now");
}

/// Round 13 (H2): a wait computed at 1 bit/s ends when the rate rises (the
/// pipeline sets it every picture tick, `RateController::stream_admit`);
/// while the pictures' share is short the budget takes no excess the share
/// will cover, and only a frame the share never covers takes it.
#[tokio::test]
async fn r13_task_a_new_rate_wakes_the_wait_and_short_share_keeps_the_budget() {
    let (mut outbox, budget) = outbox(Some(1));
    budget.on_sent(crate::screen::core::gop_stream::now_us(), 1_000, 1);
    outbox.writing = Some(writing());
    let raise = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        budget.set_rate(Some(1_000_000_000));
    };
    let both = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(outbox.ready(), raise)
    })
    .await;
    assert!(
        both.is_ok(),
        "the raised rate pays the debt in microseconds"
    );

    assert!(!budget.share_short());
    assert!(super::budget_takes_excess(&budget, true));
    budget.set_share_short(true);
    assert!(budget.share_short());
    assert!(!super::budget_takes_excess(&budget, true));
    assert!(super::budget_takes_excess(&budget, false));
}

/// Round 13 (H2): the pipeline marks the short share in the shared budget
/// when control and sound leave the pictures less than their floor, and
/// clears it when they no longer do.
#[test]
fn r13_task_stream_admit_marks_the_short_share() {
    use crate::screen::core::rate_control::{PathKind, RateController, MIN_BITRATE_BPS};
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.on_control_bytes(0, 20_000);
    rate.set_ceiling(Some(2_000_000));
    assert!(rate.stream_admit(0, true));
    assert!(!rate.shared_budget().share_short());
    rate.set_ceiling(Some(rate.control_bps() + MIN_BITRATE_BPS - 1));
    assert!(!rate.stream_admit(0, true));
    assert!(rate.shared_budget().share_short());
    rate.set_ceiling(Some(2_000_000));
    assert!(rate.stream_admit(0, true));
    assert!(!rate.shared_budget().share_short());
}
