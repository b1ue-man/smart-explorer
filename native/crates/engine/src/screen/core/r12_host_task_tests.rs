//! Task tests of review round 12 on the host's picture stream: the byte
//! budget books a picture with the small frames it absorbed once, from its
//! start (H2, Codex 12/2); the small frames' share never exceeds the user's
//! ceiling (Codex 12/5); „Verlustfrei“ on a tile stream keeps its encoder
//! (Codex 12/7); a family's new layout answers the key frame request for
//! the group before it (Codex 12/8); a small frame the budget does not take
//! waits instead of being dropped or sent on top of the debt (H5, Codex
//! 12/5). Pure: no network, no GPU, no capture.
use super::{QualityMode, StreamKind};
use crate::screen::core::negotiate::CodecPreference;
use crate::screen::core::rate_control::{PathKind, RateController};
use crate::screen::core::tiles_rate::{PictureWrite, SharedBudget, TILE_BURST_US};

/// 800 kbit/s: 20 000 bytes of saved credit (`TILE_BURST_US`).
const RATE: u64 = 800_000;

/// Round 12 (H2, Codex 12/2): a picture of 200 000 bytes sent from second 0
/// at 800 kbit/s with 20 000 bytes of credit ends at 1.8 s; a small frame's
/// excess of 1 000 bytes at 1.7 s goes with the picture's pace. Booked once
/// from the picture's start, the next picture may go when the write ended
/// (before, the frame was booked at 1.7 s, the credit earned meanwhile was
/// capped at 200 ms, the picture was booked from 1.7 s and the next one
/// waited about 1.7 s more).
#[test]
fn r12_task_picture_and_absorbed_frames_are_booked_once_from_its_start() {
    assert_eq!(TILE_BURST_US, 200_000);
    let budget = SharedBudget::default();
    assert!(!budget.absorb_while_writing(1_000), "no picture is written");
    let write = PictureWrite::start(budget.clone());
    assert_eq!(budget.credit_bits(0, RATE) / 8, 20_000);
    assert!(budget.absorb_while_writing(1_000));
    assert_eq!(budget.absorbed(), 1_000);
    budget.picture_sent(0, 200_000, Some(RATE));
    drop(write);
    assert_eq!(budget.absorbed(), 0);
    assert!(!budget.picture_writing());
    // 201 000 bytes at 100 000 bytes/s after 20 000 bytes of credit: 1.81 s.
    assert_eq!(budget.wait_us(1_810_000, RATE), 0);
    assert!(budget.admits(1_810_000, RATE));
    assert!(!budget.admits(1_700_000, RATE));

    // A write dropped before its booking books what it absorbed.
    let budget = SharedBudget::default();
    budget.set_rate(Some(RATE));
    let write = PictureWrite::start(budget.clone());
    assert!(budget.absorb_while_writing(30_000));
    drop(write);
    assert!(!budget.picture_writing());
    assert_eq!(budget.absorbed(), 0);
    assert_eq!(
        budget.credit_bits(crate::screen::core::gop_stream::now_us(), RATE),
        0
    );
}

/// Round 12 (Codex 12/5): the share the pictures leave the small control
/// frames is the measured control rate, never above the user's ceiling,
/// also right after the ceiling was lowered.
#[test]
fn r12_task_small_frames_share_stays_under_the_ceiling() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    // 125 000 bytes in the window: 1 Mbit/s of control traffic.
    rate.on_control_bytes(0, 125_000);
    assert_eq!(rate.shared_budget().reserved(), 1_000_000);
    rate.set_ceiling(Some(400_000));
    assert_eq!(rate.shared_budget().reserved(), 400_000);
    rate.on_control_bytes(10, 100);
    assert_eq!(rate.shared_budget().reserved(), 400_000);
    rate.set_ceiling(None);
    assert!(rate.shared_budget().reserved() > 1_000_000);
}

fn mode(lossless: bool, sharpness: bool, fps: u32) -> QualityMode {
    QualityMode {
        kind: StreamKind::Video,
        full_size: None,
        preview_size: None,
        full_fps: fps,
        preview_fps: 1,
        sharpness,
        lossless,
        chosen: CodecPreference::Auto,
    }
}

/// Round 12 (Codex 12/7): „Verlustfrei“ reaches the encoder only as the
/// pairing it selects, which `set_quality_mode` compares itself; a new
/// sharpness still counts, a rate alone never did.
#[test]
fn r12_task_lossless_alone_keeps_the_running_encoder() {
    assert!(mode(true, false, 30).same_but_rate(&mode(false, false, 30)));
    assert!(mode(false, false, 60).same_but_rate(&mode(true, false, 30)));
    assert!(!mode(false, true, 30).same_but_rate(&mode(false, false, 30)));
    let mode_source = include_str!("host_mode.rs").replace("\r\n", "\n");
    assert!(mode_source.contains("|| encoder(&self.choice) != encoder(&previous_choice)"));
    // The actor does not interrupt the running picture for it either.
    let actor = include_str!("host_pipeline_actor_changes.rs").replace("\r\n", "\n");
    assert!(actor.contains("!(mode.lossless && tiles)"));
}

/// Round 12 (Codex 12/8): a family's new layout answers every key frame
/// request for the groups before the one it opens, also one that arrives
/// while that group is still being encoded; a group opened for the layout
/// already takes the key frame inside it (round 12 rest).
#[test]
fn r12_task_new_layout_answers_the_request_for_the_group_before() {
    use super::super::mode::keyframe_request_stale;
    // Group 7 was the newest when the layout changed: answered below 8.
    let newest = 7u64.max(7 + 1);
    assert!(keyframe_request_stale(Some(7), newest));
    assert!(!keyframe_request_stale(Some(8), newest));
    let actor = include_str!("host_pipeline_actor_changes.rs").replace("\r\n", "\n");
    assert!(actor.contains("let newest = self.facts.gop.max(self.facts.answered_below);"));
    let rebind = actor
        .split("pub fn rebind_reference(&mut self, layout: u64) {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }\n").next())
        .expect("rebind_reference");
    assert!(rebind.contains("self.answer_requests_up_to(newest);"));
    // Round 12 rest: a group that already carries the new layout takes its
    // key frame inside it (no second group); any other opens a new one.
    assert!(rebind.contains("self.pending.layout_keyframe = Some(layout);"));
    assert!(!rebind.contains("self.pending.keyframe = true;"));
    let keyframe = actor
        .split("pub(super) fn layout_keyframe(")
        .nth(1)
        .and_then(|rest| rest.split("\n    }\n").next())
        .expect("layout_keyframe");
    assert!(keyframe.contains(".filter(|gop| gop.header().family_layout_revision == Some(layout))"));
    assert!(keyframe.contains("p.pacer.request_keyframe();"));
    assert!(keyframe.contains("None => p.request_keyframe(),"));
}

/// Round 12 (H5, Codex 12/5): a small frame whose excess finds the budget in
/// debt waits in the outbox (the newest position and statistics replacing
/// older ones) until the budget has credit; none is dropped and none goes
/// on top of the debt after a short wait.
#[test]
fn r12_task_small_frames_wait_for_the_budget() {
    let write = include_str!("host_session_send_write.rs").replace("\r\n", "\n");
    assert!(!write.contains("droppable"));
    assert!(!write.contains("SMALL_WAIT"));
    assert!(write.contains("self.outbox.queue_behind(frame);"));
    assert!(write.contains("self.outbox.behind.push_front(frame);"));
    let send = include_str!("host_session_send.rs").replace("\r\n", "\n");
    assert!(send.contains("|| !self.behind.is_empty()"));
    assert!(send.contains("const LANE_BURST_US: u64 = 1_000_000;"));
}

/// Round 12 rest (Codex 12/15): every batch of a viewer that asked for
/// receipts gets one where it is settled: held back, blocked or refused
/// while it is decided, at the end of its injection (or, on a sink that
/// queues it, once the app finished it), and stopped when the control goes
/// before its text was typed; a merged batch keeps its number in the
/// backlog's order.
#[test]
fn r12_task_input_receipts_follow_every_batch() {
    let control = include_str!("host_session_control.rs").replace("\r\n", "\n");
    assert!(control.contains("self.count_input();\n                self.input(&events).await"));
    assert!(control.contains("ViewerFrame::InputReceipts(on) => {"));
    assert!(control.contains("self.receipts_merged();\n                return Ok(());"));
    assert!(control.contains(
        "self.health.backlog.push_back(events.to_vec());\n                self.receipts_waiting(true);"
    ));
    assert!(control.contains(
        "self.health.backlog.push_back(releases);\n                self.receipts_waiting(false);"
    ));
    assert!(control.contains("self.receipts_injecting(sink.as_ref());"));
    let report = include_str!("host_session_report.rs").replace("\r\n", "\n");
    assert_eq!(report.matches("self.settle_deciding(").count(), 3);
    let inject = include_str!("host_session_inject.rs").replace("\r\n", "\n");
    assert!(inject.contains("self.poll_receipts().await?;"));
    assert!(inject.contains("self.receipts_next();\n            self.start_batch(&next).await?;"));
    assert!(inject.contains("self.receipts_applied(numbers, events).await"));
    assert_eq!(inject.matches("self.receipts_settled(").count(), 2);
    let release = include_str!("host_input_release.rs").replace("\r\n", "\n");
    assert!(release.contains("self.finish_injection().await;"));
    let receipts = include_str!("host_input_receipts.rs").replace("\r\n", "\n");
    assert!(receipts.contains("code: INPUT_STOPPED.into(),"));
}
