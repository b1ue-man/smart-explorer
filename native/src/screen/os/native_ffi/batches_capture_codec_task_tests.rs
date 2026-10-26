//! Fixtures for the one remote capture task suite. Never run locally.
use std::sync::PoisonError;
use super::openh264_codec::{codecs, tests::store_with_openh264, TEST_LOCK};
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner,
    CapturePlan, SourceKey};
use crate::screen::core::capture_grant::DelegatedCaptureCredit;
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{negotiate, CodecPreference};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

fn grant(size: FrameSize, input: u64, output: u64) -> CaptureGrant {
    let source = SourceKey::new("capture-codec-test", "source").unwrap();
    let generation = CaptureGeneration::new(1);
    let mut plan = CapturePlan::new(source.clone(), generation);
    plan.bind_contract(None).unwrap();
    for (owner, bytes) in [(CaptureOwner::SourcePixels, size.pixels() * 4),
        (CaptureOwner::CodecInput, input), (CaptureOwner::CodecOutput, output),
        (CaptureOwner::Metadata, 128)] {
        if bytes != 0 {plan.add_bytes(owner, bytes, 1, CapturePhases::ALL).unwrap();}
    }
    let credit = DelegatedCaptureCredit::from_verified_parent(source, generation,
        plan.peak_bytes(), plan.owner_limits(), ()).unwrap();
    CaptureGrant::from_delegated(plan, credit).unwrap()
}

#[test]
fn batches_task_capture_openh264_denies_i420_before_unfunded_conversion() {
    let size = FrameSize::new(64, 64);
    let grant = grant(size, 0, 1024);
    let image = CaptureImageBuilder::new(size, FrameLimits::LEGACY, &grant,
        CaptureOwner::SourcePixels).unwrap().freeze();
    let before = grant.live_bytes().unwrap();
    assert!(super::openh264_capture::admitted_i420(&grant, &image).is_err());
    assert_eq!(grant.live_bytes().unwrap(), before);
}

#[test]
fn batches_task_capture_openh264_au_keeps_credit_and_decoder_contract() {
    let fixture = "batches_task_capture_openh264_au_keeps_credit_and_decoder_contract";
    if std::env::var_os("SE_SCREEN_TASK_OPENH264").is_none() {
        eprintln!("SE_TASK_UNVERIFIED_JSON={}", serde_json::json!({"fixture": fixture,
            "reason": "SE_SCREEN_TASK_OPENH264 must name the pinned Cisco .bz2 on the remote runner"}));
        return;
    }
    if super::openh264_pins::PIN.is_none() {
        eprintln!("SE_TASK_UNVERIFIED_JSON={}", serde_json::json!({"fixture": fixture,
            "reason": "this target has no checked-in Cisco OpenH264 binary pin"}));
        return;
    }
    let _lock = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let (_directory, store) = store_with_openh264();
    let (encoders, decoders, _) = codecs(store, true);
    let registry = CodecRegistry::new(encoders, decoders);
    let choice = negotiate(&registry.encoder_offers(), &registry.decoder_offers(), CodecPreference::H264)[0];
    let size = FrameSize::new(64, 64);
    let grant = grant(size, size.pixels() * 3 / 2, 16 * 1024 * 1024);
    let mut builder = CaptureImageBuilder::new(size, FrameLimits::LEGACY, &grant,
        CaptureOwner::SourcePixels).unwrap();
    for (index, pixel) in builder.pixels_mut().chunks_exact_mut(4).enumerate() {
        pixel[..3].copy_from_slice(&[index as u8, (index / 64) as u8, 200]);
    }
    let image = builder.freeze();
    let request = StageRequest {frame_id: 1, frame_contract: None, visible_rects: None,
        keyframe: true, target_bps: 2_000_000, size, choice, encoder_epoch: 1, fps: 30};
    let mut opened = registry.open_encoder_for_stage_admitted(&request, &[], grant.clone()).unwrap();
    let encoded = opened.encoder.encode_captured(&image, &request).unwrap();
    assert!(encoded.keyframe);
    assert_eq!(encoded.records.len(), 1);
    assert!(grant.live_bytes().unwrap() > size.pixels() * 4);
    assert!(opened.encoder.encode(image.image(), true, 2_000_000).is_err());
    let header = GopHeader {family_layout_revision: None, frame_contract: None, gop_id: 1,
        kind: StreamKind::Video, codec: choice.codec, profile: choice.profile, size};
    let mut decoder = registry.open_decoder(&header).unwrap();
    assert!(decoder.decode(&FrameRecord {frame_id: 1, capture_ts_us: 0,
        flags: FLAG_KEYFRAME, payload: encoded.records[0].clone()}).unwrap());
    assert_eq!(decoder.picture().size(), size);
    drop(encoded);
    assert_eq!(grant.live_bytes().unwrap(), size.pixels() * 4);
    drop(image);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
