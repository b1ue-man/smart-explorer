//! Prepared regression cases; execution belongs to the single remote suite.
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureOwner, CapturePlan, SourceKey},
    capture_image::CaptureImageBuilder,
    capture_plan::CapturePhases,
    frame_budget::{self, FrameCapabilities, FrameContract, FrameLimits, MAX_WORKING_BYTES},
    frame_memory::{CapacityCeiling, MemoryPool},
    frame_size_caps::PacketBudget,
    host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan},
    image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole, LocalImagePolicy},
    pixel_convert::{BgraImage, FrameSize},
    tiles_codec::{TileDecoder, TileEncoder, MAX_TILES_RECORD_BYTES},
    wire_binary::{self, FrameReadBudget, FrameRecord, GopHeader, StreamKind},
    wire_hello::{ScreenClientHello, SessionMode},
    worker_capture_wire::OutputWire,
};
use std::{io, sync::Arc};
const MIB: u64 = 1024 * 1024;
fn hello() -> ScreenClientHello {
    serde_json::from_value(serde_json::json!({"protocol_version":1,"relation_kind":"direct","relation_id":"fixture",
        "device_id":"viewer","device_name":"viewer","public_key":"key","fingerprint":"pin","node_id":"node",
        "client_nonce":"nonce","proof":"proof","mode":"preview","source":"synthetic",
        "decoders":[{"codec":"tiles"}],"display_size":{"width":320,"height":320},"max_fps":1})).unwrap()
}
fn preview() -> u64 {
    frame_budget::requested_bytes(&hello(), 64 * MIB).unwrap()
}

#[test]
fn batches_task_b6_session_budget_two_android_previews_share_the_real_process_ceiling() {
    let pool = Arc::new(MemoryPool::default());
    let ceiling = CapacityCeiling::bounded(64 * MIB);
    pool.observe_capacity_ceiling(ceiling).unwrap();
    let requested = preview();
    assert!(requested < 32 * MIB);
    let first = Arc::new(pool.reserve_with_ceiling(None, ceiling, requested).unwrap());
    let second = Arc::new(pool.reserve_with_ceiling(None, ceiling, requested).unwrap());
    assert_eq!(first.bytes(), requested);
    assert_eq!(second.bytes(), requested);
    assert_eq!(pool.committed_bytes(), 2 * requested);
    for lease in [&first, &second] {
        let limits = frame_budget::legacy_limits(lease.bytes(), None).unwrap();
        assert!(limits.check(FrameSize::new(320, 320)).is_ok());
        assert!(PacketBudget::legacy_memory(limits).unwrap().1 <= lease.bytes());
    }
    let tail = pool.reserve_with_ceiling(None, ceiling, 64 * MIB).unwrap();
    assert_eq!(tail.bytes(), 64 * MIB - 2 * requested);
    assert!(pool.reserve_with_ceiling(None, ceiling, MIB).is_err());
    let renderer = first.clone();
    drop(first);
    drop(second);
    drop(tail);
    assert_eq!(
        pool.committed_bytes(),
        requested,
        "the actual last image owner keeps its lease"
    );
    drop(renderer);
    assert_eq!(pool.committed_bytes(), 0);
}

#[test]
fn batches_task_b6_session_budget_preview_contract_is_small_and_full_request_is_independent() {
    let preview = hello();
    let requested = frame_budget::requested_bytes(&preview, 128 * MIB).unwrap();
    let caps = FrameCapabilities::admitted(FrameSize::new(4096, 4096), requested).unwrap();
    let caps = frame_budget::preview_capabilities(caps, &preview).unwrap();
    let contract = FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap();
    assert_eq!(contract.limits.max_size, FrameSize::new(320, 320));
    assert!(contract.limits.check(FrameSize::new(320, 320)).is_ok());
    assert!(contract.limits.check(FrameSize::new(1920, 1080)).is_err());
    let mut full = preview;
    full.mode = SessionMode::View;
    full.display_size = Some(FrameSize::new(1920, 1080));
    assert_eq!(
        frame_budget::requested_bytes(&full, 128 * MIB).unwrap(),
        128 * MIB
    );
    let full_caps = FrameCapabilities::admitted(FrameSize::new(4096, 4096), 128 * MIB).unwrap();
    assert!(frame_budget::preview_capabilities(full_caps, &full)
        .unwrap()
        .frames
        .check(FrameSize::new(1280, 720))
        .is_ok());
    assert_eq!(FrameContract::negotiate(Some(caps), None).unwrap(), None);
    assert!(frame_budget::legacy_limits(0, None).is_err());
    assert!(frame_budget::legacy_limits(32, None).is_err());
    assert!(frame_budget::legacy_limits(64 * MIB, Some(FrameSize::new(0, 1))).is_err());
    assert_eq!(
        frame_budget::legacy_limits(MAX_WORKING_BYTES, None).unwrap(),
        FrameLimits::LEGACY
    );
}

#[test]
fn batches_task_b6_session_budget_legacy_host_plan_worker_resize_and_image_retirement_stay_bound() {
    let pool = Arc::new(MemoryPool::default());
    let ceiling = CapacityCeiling::bounded(128 * MIB);
    let admission = pool
        .admit_session(None, ceiling, preview(), "legacy-small")
        .unwrap();
    admission.bind_contract(None).unwrap();
    let limits = frame_budget::legacy_limits(admission.working_bytes(), None).unwrap();
    let output = HostOutputPlan::new(
        HostContract::Legacy,
        limits.fit(limits.max_size),
        HostCodecPlan::Tiles,
    )
    .unwrap()
    .with_legacy_limits(limits)
    .unwrap();
    let restored = OutputWire::from_plan(&output).plan().unwrap();
    assert_eq!(restored.contract(), HostContract::Legacy);
    assert_eq!(restored.limits(), limits);
    let resized = restored
        .reconfigure(FrameSize::new(160, 90), HostCodecPlan::Tiles)
        .unwrap();
    assert_eq!(
        resized.limits(),
        limits,
        "raw capture/codec reconfiguration preserves local limits"
    );
    assert!(restored
        .reconfigure(FrameSize::new(4096, 2304), HostCodecPlan::Tiles)
        .is_err());
    let source = SourceKey::new(admission.session(), "synthetic").unwrap();
    let native = FrameSize::new(640, 480);
    let mut plan = CapturePlan::new(source.clone(), CaptureGeneration::new(1));
    plan.add_bytes(
        CaptureOwner::SourcePixels,
        native.pixels() * 4,
        1,
        CapturePhases::ALL,
    )
    .unwrap();
    output
        .append_to(&mut plan, HostInputPixels::SeparateImage)
        .unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let image = CaptureImageBuilder::new(
        native,
        FrameLimits::RESOURCE,
        &grant,
        CaptureOwner::SourcePixels,
    )
    .unwrap()
    .freeze();
    let mut wrong = CapturePlan::new(
        SourceKey::new("wrong-session", "synthetic").unwrap(),
        CaptureGeneration::new(1),
    );
    wrong
        .add_bytes(CaptureOwner::SourcePixels, 4, 1, CapturePhases::ALL)
        .unwrap();
    assert!(admission.authority().approve(wrong).is_err());
    assert!(admission
        .bind_contract(Some(FrameContract {
            version: 2,
            limits,
            hybrid_regions: false
        }))
        .is_err());
    let held = pool.committed_bytes();
    drop(grant);
    drop(admission);
    assert_eq!(
        pool.committed_bytes(),
        held,
        "session close is not image completion"
    );
    drop(image);
    assert_eq!(pool.committed_bytes(), 0);
}

#[test]
fn batches_task_b6_session_budget_login_image_claims_compete_with_preview_leases() {
    let pool = Arc::new(MemoryPool::default());
    let ceiling = CapacityCeiling::bounded(64 * MIB);
    let first = pool.reserve_with_ceiling(None, ceiling, preview()).unwrap();
    let second = pool.reserve_with_ceiling(None, ceiling, preview()).unwrap();
    let image_bytes = 640 * 480 * 4;
    let limits = ImageOwnerLimits::new(2 * image_bytes)
        .unwrap()
        .with_role(ImageRole::ViewerInput, image_bytes, 2)
        .unwrap();
    let images = ImageAdmission::in_test_pool(
        pool.clone(),
        LocalImagePolicy {
            facts: None,
            ceiling,
        },
        limits,
    );
    let login = images
        .try_claim(ImageRole::ViewerInput, image_bytes)
        .unwrap();
    assert_eq!(
        pool.committed_bytes(),
        first.bytes() + second.bytes() + image_bytes
    );
    let before = pool.committed_bytes();
    let oversized = ImageOwnerLimits::new(32 * MIB)
        .unwrap()
        .with_role(ImageRole::ViewerInput, 32 * MIB, 1)
        .unwrap();
    let denied = ImageAdmission::in_test_pool(
        pool.clone(),
        LocalImagePolicy {
            facts: None,
            ceiling,
        },
        oversized,
    );
    assert!(denied.try_claim(ImageRole::ViewerInput, 32 * MIB).is_err());
    assert_eq!(pool.committed_bytes(), before);
    drop(first);
    drop(second);
    assert_eq!(pool.committed_bytes(), image_bytes);
    drop(login);
    assert_eq!(pool.committed_bytes(), 0);
}

#[tokio::test]
async fn batches_task_b6_session_budget_legacy_record_length_is_rejected_before_payload_read() {
    let limits = frame_budget::legacy_limits(preview(), None).unwrap();
    let packet = PacketBudget::for_receive(None, Some(limits)).unwrap();
    let header = GopHeader {
        gop_id: 1,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Preview,
        codec: crate::screen::core::negotiate::CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(320, 320),
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&((packet.record_bytes + 1) as u32).to_be_bytes());
    bytes.extend_from_slice(&1u64.to_be_bytes());
    bytes.extend_from_slice(&1u64.to_be_bytes());
    bytes.push(wire_binary::FLAG_KEYFRAME);
    let error = wire_binary::read_frame_record_admitted(
        &mut bytes.as_slice(),
        &header,
        &mut FrameReadBudget::default(),
        packet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "there is no payload; reading it would return UnexpectedEof"
    );
}

#[test]
fn batches_task_b6_session_budget_v184_whole_tile_records_fit_the_small_legacy_receiver() {
    let size = FrameSize::new(320, 320);
    let limits = frame_budget::legacy_limits(preview(), None).unwrap();
    let packet = PacketBudget::for_receive(None, Some(limits)).unwrap();
    let mut image = BgraImage::new(size).unwrap();
    let mut noise = 1u32;
    for y in 0..size.height {
        for pixel in image.row_mut(y).chunks_exact_mut(4) {
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            pixel.copy_from_slice(&[noise as u8, (noise >> 8) as u8, (noise >> 16) as u8, 255]);
        }
    }
    let mut encoder = TileEncoder::new(size).unwrap();
    let records = encoder
        .encode(&image.view(), true, MAX_TILES_RECORD_BYTES)
        .unwrap();
    assert_eq!(
        records.len(),
        1,
        "released v1 packs this picture as one whole record"
    );
    assert!(
        records[0].len() > 64 * 64 * 4,
        "this must cover more than the v2 minimum tile record"
    );
    assert!(records[0].len() <= packet.record_bytes);
    let mut decoder = TileDecoder::new_with_limits(size, limits).unwrap();
    let record = FrameRecord {
        frame_id: 1,
        capture_ts_us: 1,
        flags: wire_binary::FLAG_KEYFRAME,
        payload: records.into_iter().next().unwrap(),
    };
    assert!(decoder.apply_frame_record(&record).unwrap());
    assert_eq!(decoder.image().data(), image.data());
    let header = GopHeader {
        gop_id: 1,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Preview,
        codec: crate::screen::core::negotiate::CodecKind::Tiles,
        profile: None,
        size,
    };
    assert_eq!(header.encode().unwrap()[4], wire_binary::GOP_VERSION);
}
