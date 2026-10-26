//! B6 bounds are exercised with a deliberately small negotiated contract.
use super::*;
use crate::screen::core::frame_size_caps::{validate_encoded_budget, PacketBudget};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::wire_binary::{self, FrameReadBudget, GopHeader, StreamKind};
use crate::screen::core::worker_wire;

fn contract() -> FrameContract {
    FrameContract {
        version: FRAME_PROTOCOL,
        limits: FrameLimits {
            max_size: FrameSize::new(128, 128),
            max_pixels: 128 * 128,
            working_bytes: 4 * 1024 * 1024,
        },
        hybrid_regions: true,
    }
}
fn header(contract: Option<FrameContract>) -> GopHeader {
    GopHeader {
        gop_id: 1,
        frame_contract: contract,
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec: CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(128, 128),
    }
}
fn record_head(length: usize, frame: u64) -> Vec<u8> {
    let mut head = Vec::new();
    head.extend_from_slice(&u32::try_from(length).unwrap().to_be_bytes());
    head.extend_from_slice(&frame.to_be_bytes());
    head.extend_from_slice(&100u64.to_be_bytes());
    head.push(wire_binary::FLAG_KEYFRAME | wire_binary::FLAG_MORE);
    head
}
fn worker_head(tag: u8, length: usize) -> Vec<u8> {
    let mut head = vec![tag];
    head.extend_from_slice(&u32::try_from(length).unwrap().to_be_bytes());
    head
}

#[test]
fn batches_task_b6_negotiation_intersects_dimensions_pixels_and_memory() {
    let offer = FrameCapabilities {
        version: FRAME_PROTOCOL,
        frames: contract().limits,
        texture_size: FrameSize::new(128, 128),
        hybrid_regions: true,
    };
    let viewer = FrameCapabilities {
        texture_size: FrameSize::new(96, 64),
        frames: FrameLimits {
            max_pixels: 96 * 64,
            working_bytes: 2 * 1024 * 1024,
            ..offer.frames
        },
        hybrid_regions: false,
        ..offer
    };
    let negotiated = FrameContract::negotiate(Some(offer), Some(viewer))
        .unwrap()
        .unwrap();
    assert_eq!(negotiated.limits.max_size, FrameSize::new(96, 64));
    assert_eq!(negotiated.limits.max_pixels, 96 * 64);
    assert_eq!(negotiated.limits.working_bytes, 2 * 1024 * 1024);
    assert!(!negotiated.hybrid_regions);
    assert!(negotiated.limits.check(FrameSize::new(96, 64)).is_ok());
    assert!(negotiated.limits.check(FrameSize::new(97, 64)).is_err());
    let narrowed = offer.with_working_budget(2 * 1024 * 1024).unwrap();
    assert!(narrowed.frames.max_pixels <= offer.frames.max_pixels);
    assert_eq!(narrowed.frames.working_bytes, 2 * 1024 * 1024);
}

#[test]
fn batches_task_b6_legacy_keeps_its_own_contract() {
    let offer = FrameCapabilities::resources(FrameSize::new(128, 128));
    assert_eq!(FrameContract::negotiate(None, Some(offer)).unwrap(), None);
    assert_eq!(FrameContract::negotiate(Some(offer), None).unwrap(), None);
    assert!(FrameLimits::LEGACY
        .check(FrameSize::new(3840, 2160))
        .is_ok());
    let legacy = header(None);
    assert_eq!(legacy.encode().unwrap()[4], wire_binary::GOP_VERSION);
    assert_eq!(
        legacy.record_limit(),
        crate::screen::core::tiles_codec::MAX_TILES_RECORD_BYTES
    );
    assert!(header(Some(contract())).record_limit() < legacy.record_limit());
    assert_eq!(
        PacketBudget::for_contract(None).unwrap().record_bytes,
        wire_binary::MAX_RECORD_BYTES
    );
}

#[test]
fn batches_task_b6_producer_rejects_record_count_and_payload_over_budget() {
    let contract = contract();
    let budget = PacketBudget::for_contract(Some(contract)).unwrap();
    let too_big = vec![vec![0; budget.record_bytes + 1]];
    assert!(validate_encoded_budget(Some(contract), CodecKind::H264, &too_big).is_err());
    assert!(validate_encoded_budget(None, CodecKind::H264, &too_big).is_ok());
    assert!(validate_encoded_budget(
        Some(contract),
        CodecKind::Tiles,
        &vec![Vec::new(); budget.records + 1]
    )
    .is_err());
    let records =
        vec![vec![0; budget.record_bytes]; budget.encoded_bytes / budget.record_bytes + 1];
    assert!(
        records.len() <= budget.records,
        "fixture must isolate bytes from record count"
    );
    assert!(validate_encoded_budget(Some(contract), CodecKind::H264, &records).is_err());
}

#[tokio::test]
async fn batches_task_b6_record_reader_rejects_length_before_payload_read() {
    let header = header(Some(contract()));
    let bytes = record_head(header.record_limit() + 1, 1);
    // No payload exists: an attempted payload read would return UnexpectedEof.
    let error = wire_binary::read_frame_record(&mut bytes.as_slice(), &header)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("limit of its codec"));
}

#[tokio::test]
async fn batches_task_b6_record_reader_limits_whole_transaction() {
    let header = header(Some(contract()));
    let limit = PacketBudget::for_contract(header.frame_contract).unwrap();
    let mut budget = FrameReadBudget::default();
    for _ in 0..limit.records {
        let bytes = record_head(0, 1);
        assert!(wire_binary::read_frame_record_budgeted(
            &mut bytes.as_slice(),
            &header,
            &mut budget
        )
        .await
        .unwrap()
        .is_some());
    }
    let bytes = record_head(0, 1);
    let error =
        wire_binary::read_frame_record_budgeted(&mut bytes.as_slice(), &header, &mut budget)
            .await
            .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("transaction"));
    let bytes = record_head(0, 2);
    assert!(
        wire_binary::read_frame_record_budgeted(&mut bytes.as_slice(), &header, &mut budget)
            .await
            .unwrap()
            .is_some()
    );

    let mut budget = FrameReadBudget::default();
    let per_record = limit.record_bytes + wire_binary::RECORD_HEADER_BYTES;
    let accepted = limit.encoded_bytes / per_record;
    assert!(
        accepted + 1 <= limit.records,
        "fixture must isolate aggregate bytes"
    );
    for _ in 0..accepted {
        let mut bytes = record_head(limit.record_bytes, 3);
        bytes.resize(per_record, 0);
        assert!(wire_binary::read_frame_record_budgeted(
            &mut bytes.as_slice(),
            &header,
            &mut budget
        )
        .await
        .unwrap()
        .is_some());
    }
    let bytes = record_head(limit.record_bytes, 3);
    let error =
        wire_binary::read_frame_record_budgeted(&mut bytes.as_slice(), &header, &mut budget)
            .await
            .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("transaction"));
}

#[test]
fn batches_task_b6_worker_rejects_negotiated_packet_before_payload_read() {
    let contract = contract();
    let budget = PacketBudget::for_contract(Some(contract)).unwrap();
    for (tag, bound) in [
        (worker_wire::RAW, budget.raw_bytes),
        (worker_wire::ENCODED, budget.encoded_bytes),
    ] {
        let bytes = worker_head(tag, bound + 1);
        let error =
            worker_wire::read_message_with_limits(&mut bytes.as_slice(), Some(contract.limits))
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("PRODUCE working memory"));
        let legacy_error = worker_wire::read_message(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(legacy_error.kind(), io::ErrorKind::UnexpectedEof);
    }
}

#[test]
fn batches_task_b6_worker_reference_cannot_raise_negotiated_budget() {
    let contract = contract();
    let budget = PacketBudget::for_contract(Some(contract)).unwrap();
    let mut bytes = worker_head(worker_wire::REFERENCE, 42 + budget.raw_bytes + 1);
    bytes.extend_from_slice(&1u64.to_be_bytes());
    bytes.push(1);
    for side in [
        FrameLimits::LEGACY.max_size.width,
        FrameLimits::LEGACY.max_size.height,
    ] {
        bytes.extend_from_slice(&side.to_be_bytes());
    }
    bytes.extend_from_slice(&FrameLimits::LEGACY.max_pixels.to_be_bytes());
    bytes.extend_from_slice(&FrameLimits::LEGACY.working_bytes.to_be_bytes());
    bytes.extend_from_slice(&0u64.to_be_bytes());
    bytes.push(worker_wire::RAW);
    assert_eq!(bytes.len(), 5 + 42);
    let error = worker_wire::read_message_with_limits(&mut bytes.as_slice(), Some(contract.limits))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error
        .to_string()
        .contains("referenced worker reply exceeds"));
}
