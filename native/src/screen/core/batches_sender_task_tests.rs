use super::*;

fn admit(flow: &SenderBudget, now: u64) -> Option<EncodePermit> {
    flow.admit(100_000_000, PathKind::Relay, 100_000, now, true)
}

#[test]
fn batches_task_b6_sender_reset_drop_and_ack_preserve_sibling_debt() {
    let first = SenderBudget::for_peer("batches-b6-sibling".into());
    let second = SenderBudget::for_peer("batches-b6-sibling".into());
    admit(&first, 1).unwrap().sent(7, 10, 500_000);
    assert!(admit(&second, 2).is_none());
    second.reset();
    assert!(admit(&second, 3).is_none());
    drop(second);
    let third = SenderBudget::for_peer("batches-b6-sibling".into());
    assert!(admit(&third, 4).is_none());
    third.ack(7, 10);
    first.ack(6, 100);
    assert!(admit(&third, 5).is_none());
    first.ack(7, 10);
    assert!(admit(&third, 6).is_some());
}

#[test]
fn batches_task_b6_sender_abandoned_encode_permit_releases_capacity_fairly() {
    let first = SenderBudget::for_peer("batches-b6-fair".into());
    let second = SenderBudget::for_peer("batches-b6-fair".into());
    let permit = admit(&first, 1).unwrap();
    assert!(admit(&second, 2).is_none());
    drop(permit);
    assert!(admit(&first, 3).is_none(), "recently waiting sibling owns the next turn");
    let permit = admit(&second, 4).unwrap();
    assert!(admit(&first, 5).is_none());
    drop(permit);
    assert!(admit(&first, 6).is_some());
}

#[test]
fn batches_task_b6_sender_control_reservation_is_peer_shared_and_survives_flow_reset() {
    let peer = "batches-b6-control".to_string();
    let flow = SenderBudget::for_peer(peer.clone());
    let bytes = SenderBudget::control_reserve_bytes() as usize;
    let control = reserve_control(peer.clone(), bytes).unwrap();
    assert!(reserve_control(peer.clone(), 1).is_none());
    flow.reset();
    drop(flow);
    assert!(reserve_control(peer.clone(), 1).is_none());
    assert!(reserve_control("batches-b6-independent".into(), bytes).is_some());
    drop(control);
    assert!(reserve_control(peer, bytes).is_some());
    assert_eq!(SenderBudget::control_reserve_bytes() + super::super::pacing::RELAY_GUARD_BYTES,
        super::super::pacing::RELAY_QUEUE_BYTES);
}

#[test]
fn batches_task_b6_rate_path_change_resets_loss_and_keeps_native_source_pixels() {
    use super::super::rate_control::{RateController, quality_for, START_DIRECT_BPS, START_RELAY_BPS};
    use super::super::pixel_convert::FrameSize;
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.on_loss_counters(100, 0);
    rate.on_loss_counters(200, 100);
    assert!(rate.target_bps() < START_DIRECT_BPS);
    rate.set_path(PathKind::Relay);
    assert_eq!(rate.target_bps(), START_RELAY_BPS);
    assert_eq!(rate.on_loss_counters(200, 100), None);
    rate.set_path(PathKind::Direct);
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    let source = FrameSize::new(3840, 2160);
    let quality = quality_for(150_000, source, 60);
    assert_eq!(quality.size, source);
    assert_eq!(quality.fps, 1);
}
