//! V7 mathematical oracles and admitted capture setup; no codec is substituted.
use super::{
    capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SourceKey},
    capture_plan::CapturePhases,
    capture_test_support,
    codec_registry::CodecRegistry,
    decoded_frame::DecodedFrame,
    frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL},
    host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan},
    host_stage::StageRequest,
    negotiate::{negotiate, CodecPreference},
    pixel_convert::{BgraImage, FrameSize},
    quality_acceptance_metrics::{self as metrics, measure},
    wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME, FLAG_MORE},
};

pub fn capture(size: FrameSize, phase: u64) -> BgraImage {
    let mut image = BgraImage::new(size).unwrap();
    for (index, pixel) in image.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = (index as u32 % size.width, index as u32 / size.width);
        let moving = x / 16 == phase as u32 % (size.width / 16) && y > size.height / 3;
        let stroke = x % 11 == 0 || y % 17 == 0 || (x + y) % 23 == 0;
        pixel.copy_from_slice(if moving {
            &[240, 80, 40, 255]
        } else if stroke {
            &[8, 8, 8, 255]
        } else {
            &[230, 230, 230, 255]
        });
    }
    image
}

pub fn grant(request: &StageRequest) -> CaptureGrant {
    let admission = capture_test_support::session(request.frame_contract);
    let mut plan = CapturePlan::new(
        SourceKey::new(admission.session(), "quality-capture").unwrap(),
        CaptureGeneration::new(1),
    );
    plan.bind_contract(request.frame_contract).unwrap();
    plan.add_bgra(
        CaptureOwner::SourcePixels,
        request.size,
        1,
        CapturePhases::ALL,
    )
    .unwrap();
    HostOutputPlan::new(
        request
            .frame_contract
            .map_or(HostContract::Legacy, HostContract::Versioned),
        request.size,
        HostCodecPlan::OpenH264,
    )
    .unwrap()
    .append_to(&mut plan, HostInputPixels::BorrowedSource)
    .unwrap();
    admission.authority().approve(plan).unwrap()
}
pub fn request(
    registry: &CodecRegistry,
    size: FrameSize,
    bps: u64,
    hybrid: bool,
) -> StageRequest {
    let choice = negotiate(
        &registry.encoder_offers(),
        &registry.decoder_offers(),
        CodecPreference::H264,
    )[0];
    assert_eq!(
        choice.codec,
        crate::screen::core::negotiate::CodecKind::H264
    );
    StageRequest {
        frame_id: 1,
        frame_contract: hybrid.then_some(FrameContract {
            version: FRAME_PROTOCOL,
            limits: FrameLimits {
                max_size: size,
                max_pixels: size.pixels(),
                working_bytes: 64 * 1024 * 1024,
            },
            hybrid_regions: true,
        }),
        visible_rects: None,
        keyframe: true,
        target_bps: bps,
        size,
        choice,
        encoder_epoch: 1,
        fps: 30,
    }
}
pub fn header(request: &StageRequest) -> GopHeader {
    GopHeader {
        family_layout_revision: None,
        frame_contract: request.frame_contract,
        gop_id: 1,
        kind: StreamKind::Video,
        codec: request.choice.codec,
        profile: request.choice.profile,
        size: request.size,
    }
}
pub fn record(frame_id: u64, keyframe: bool, more: bool, payload: Vec<u8>) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: frame_id * 33_333,
        flags: if keyframe { FLAG_KEYFRAME } else { 0 } | if more { FLAG_MORE } else { 0 },
        payload,
    }
}
fn constant(value: u8) -> BgraImage {
    let mut image = BgraImage::new(FrameSize::new(22, 22)).unwrap();
    for pixel in image.data_mut().chunks_exact_mut(4) {
        pixel[..3].fill(value);
    }
    image
}
