//! COM lifetime and noncoalescing MF output fixtures for remote Windows CI.
use windows::core::{Interface, IUnknown};
use windows::Win32::Media::MediaFoundation::IMFMediaBuffer;
use super::{mf_buffer_lease, mf_capture_media, mf_capture_output};
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner,
    CapturePlan, SourceKey};
use crate::screen::core::capture_grant::DelegatedCaptureCredit;
use crate::screen::core::capture_image::AdmittedBytes;
use crate::screen::core::capture_plan::CapturePhases;

fn grant(owner: CaptureOwner, bytes: u64) -> CaptureGrant {
    let source = SourceKey::new("mf-capture-test", "source").unwrap();
    let generation = CaptureGeneration::new(1);
    let mut plan = CapturePlan::new(source.clone(), generation);
    plan.bind_contract(None).unwrap();
    plan.add_bytes(owner, bytes, 1, CapturePhases::ALL).unwrap();
    let credit = DelegatedCaptureCredit::from_verified_parent(source, generation,
        plan.peak_bytes(), plan.owner_limits(), ()).unwrap();
    CaptureGrant::from_delegated(plan, credit).unwrap()
}

#[test]
fn batches_task_capture_mf_buffer_credit_lasts_through_final_com_alias() {
    if !media_foundation("batches_task_capture_mf_buffer_credit_lasts_through_final_com_alias") {return;}
    let grant = grant(CaptureOwner::CodecInput, 16);
    let buffer = mf_buffer_lease::create(&grant, CaptureOwner::CodecInput, 16).unwrap();
    let sample = mf_capture_media::sample_with_buffer(&buffer).unwrap();
    let retained = unsafe {sample.GetBufferByIndex(0).unwrap()};
    let identity: IUnknown = retained.cast().unwrap();
    drop(sample);
    drop(buffer);
    drop(retained);
    assert_eq!(grant.live_bytes().unwrap(), 16);
    assert!(mf_buffer_lease::create(&grant, CaptureOwner::CodecInput, 16).is_err());
    let retained: IMFMediaBuffer = identity.cast().unwrap();
    drop(identity);
    assert_eq!(unsafe {retained.GetMaxLength().unwrap()}, 16);
    drop(retained);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    assert!(mf_buffer_lease::create(&grant, CaptureOwner::CodecInput, 16).is_ok());
}

fn fill(buffer: &IMFMediaBuffer, bytes: &[u8]) {
    let mut data = std::ptr::null_mut();
    unsafe {
        buffer.Lock(&mut data, None, None).unwrap();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        buffer.Unlock().unwrap();
        buffer.SetCurrentLength(bytes.len() as u32).unwrap();
    }
}

#[test]
fn batches_task_capture_mf_copies_multiple_buffers_with_one_fixed_au() {
    if !media_foundation("batches_task_capture_mf_copies_multiple_buffers_with_one_fixed_au") {return;}
    let grant = grant(CaptureOwner::CodecOutput, 32);
    let first = mf_buffer_lease::create(&grant, CaptureOwner::CodecOutput, 8).unwrap();
    let second = mf_buffer_lease::create(&grant, CaptureOwner::CodecOutput, 8).unwrap();
    fill(&first, b"first");
    fill(&second, b"second");
    let sample = mf_capture_media::sample_with_buffer(&first).unwrap();
    unsafe {sample.AddBuffer(&second).unwrap()};
    let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, 16).unwrap();
    mf_capture_output::append_sample(&mut bytes, &sample).unwrap();
    assert_eq!(bytes.as_slice(), b"firstsecond");
    assert!(mf_capture_output::append_sample(&mut bytes, &sample).is_err());
    assert_eq!(bytes.as_slice(), b"firstsecond");
    drop(sample);
    drop(first);
    drop(second);
    assert_eq!(grant.live_bytes().unwrap(), 16);
    drop(bytes);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}

fn media_foundation(fixture: &str) -> bool {
    match super::mf_common::platform() {
        Ok(_) => true,
        Err(error) => {
            unverified(fixture, &format!("Media Foundation mfplat.dll/MFStartup unavailable on this Windows runner: {error}"));
            false
        }
    }
}

fn unverified(fixture: &str, reason: &str) {
    eprintln!("SE_TASK_UNVERIFIED_JSON={}", serde_json::json!({"fixture": fixture, "reason": reason}));
}

#[test]
fn batches_task_capture_mf_sync_admitted_roundtrip() {
    encoder_roundtrip(false, "batches_task_capture_mf_sync_admitted_roundtrip");
}

#[test]
fn batches_task_capture_mf_async_admitted_roundtrip() {
    encoder_roundtrip(true, "batches_task_capture_mf_async_admitted_roundtrip");
}

fn encoder_roundtrip(hardware: bool, fixture: &str) {
    use crate::screen::core::capture_image::CaptureImageBuilder;
    use crate::screen::core::codec_registry::CodecRegistry;
    use crate::screen::core::frame_budget::FrameLimits;
    use crate::screen::core::host_stage::StageRequest;
    use crate::screen::core::negotiate::{negotiate, CodecPreference};
    use crate::screen::core::pixel_convert::FrameSize;
    use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};
    if !media_foundation(fixture) {return;}
    let (encoders, decoders) = super::mf_common::codecs();
    let Some(factory) = encoders.into_iter().find(|factory| factory.offers().iter().any(|offer| offer.hardware == hardware)) else {
        unverified(fixture, if hardware {"no asynchronous hardware H.264 MFT registered on this Windows runner"}
            else {"no synchronous software H.264 MFT registered on this Windows runner"});
        return;
    };
    let offers: Vec<_> = decoders.iter().flat_map(|factory| factory.offers()).collect();
    let choices = negotiate(&factory.offers(), &offers, CodecPreference::H264);
    let Some(choice) = choices.first().copied() else {
        unverified(fixture, "no compatible Media Foundation H.264 decoder registered on this Windows runner");
        return;
    };
    let registry = CodecRegistry::new(vec![factory], decoders);
    let size = FrameSize::new(640, 360);
    let source = SourceKey::new("mf-capture-roundtrip", "source").unwrap();
    let generation = CaptureGeneration::new(1);
    let mut plan = CapturePlan::new(source.clone(), generation);
    plan.bind_contract(None).unwrap();
    for (owner, bytes) in [(CaptureOwner::SourcePixels, size.pixels() * 4),
        (CaptureOwner::CodecInput, size.pixels() * 3),
        (CaptureOwner::CodecOutput, 80 * 1024 * 1024), (CaptureOwner::Metadata, 128)] {
        plan.add_bytes(owner, bytes, 1, CapturePhases::ALL).unwrap();
    }
    let credit = DelegatedCaptureCredit::from_verified_parent(source, generation,
        plan.peak_bytes(), plan.owner_limits(), ()).unwrap();
    let grant = CaptureGrant::from_delegated(plan, credit).unwrap();
    let image = CaptureImageBuilder::new(size, FrameLimits::LEGACY, &grant,
        CaptureOwner::SourcePixels).unwrap().freeze();
    let request = StageRequest {frame_id: 1, frame_contract: None, visible_rects: None,
        keyframe: true, target_bps: 2_000_000, size, choice, encoder_epoch: 1, fps: 30};
    let mut opened = registry.open_encoder_for_stage_admitted(&request, &[], grant.clone()).unwrap();
    assert_eq!(opened.hardware, hardware);
    let encoded = opened.encoder.encode_captured(&image, &request).unwrap();
    assert!(encoded.keyframe);
    assert_eq!(encoded.records.len(), 1);
    assert!(grant.live_bytes().unwrap() >= size.pixels() * 4 + encoded.records[0].len() as u64);
    assert!(opened.encoder.encode(image.image(), true, request.target_bps).is_err());
    let header = GopHeader {family_layout_revision: None, frame_contract: None, gop_id: 1,
        kind: StreamKind::Video, codec: choice.codec, profile: choice.profile, size};
    let mut decoder = registry.open_decoder(&header).unwrap();
    assert!(decoder.decode(&FrameRecord {frame_id: 1, capture_ts_us: 0,
        flags: FLAG_KEYFRAME, payload: encoded.records[0].clone()}).unwrap());
    assert_eq!(decoder.picture().size(), size);
}
