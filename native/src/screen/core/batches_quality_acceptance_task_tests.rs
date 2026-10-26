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

pub(crate) fn capture(size: FrameSize, phase: u64) -> BgraImage {
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

pub(crate) fn grant(request: &StageRequest) -> CaptureGrant {
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
pub(crate) fn request(
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
pub(crate) fn header(request: &StageRequest) -> GopHeader {
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
pub(crate) fn record(frame_id: u64, keyframe: bool, more: bool, payload: Vec<u8>) -> FrameRecord {
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

#[test]
fn batches_task_b6_quality_metrics_match_independent_analytic_oracles() {
    let black = constant(0);
    let white = constant(255);
    let same = measure(&black, &black).unwrap();
    assert_eq!(same.different_pixels, 0);
    assert_eq!(same.ssim_luma, 1.0);
    assert!(same.psnr_infinite && same.psnr_rgb_db.is_none());
    let opposite = measure(&black, &white).unwrap();
    assert_eq!(opposite.rgb_squared_error, 22 * 22 * 3 * 255 * 255);
    assert_eq!(opposite.psnr_rgb_db, Some(0.0));
    let expected = (0.01_f64 * 255.0).powi(2) / (255.0_f64.powi(2) + (0.01_f64 * 255.0).powi(2));
    assert!((opposite.ssim_luma - expected).abs() < 1e-12);
    let mut one = black.clone();
    one.data_mut()[0] = 1;
    let single = measure(&black, &one).unwrap();
    assert_eq!(single.different_pixels, 1);
    assert_eq!(single.rgb_squared_error, 1);
    assert!(
        (single.psnr_rgb_db.unwrap() - 10.0 * (255.0_f64.powi(2) * 22.0 * 22.0 * 3.0).log10())
            .abs()
            < 1e-12
    );
    assert!(!metrics::no_codec_regression(&same, &single));
    // One valid window: symmetry fixes the means at 100 and 155. Contrast
    // inversion fixes covariance at -variance, independently of convolution.
    let mut ramp = BgraImage::new(FrameSize::new(11, 11)).unwrap();
    let mut inverse = ramp.clone();
    for (index, (a, b)) in ramp
        .data_mut()
        .chunks_exact_mut(4)
        .zip(inverse.data_mut().chunks_exact_mut(4))
        .enumerate()
    {
        let value = ((index % 11 + index / 11) * 10) as u8;
        a[..3].fill(value);
        b[..3].fill(255 - value);
    }
    let (mut weight_sum, mut weighted_second) = (0.0, 0.0);
    for offset in -5..=5 {
        let squared = f64::from(offset * offset);
        let weight = (-squared / (2.0 * 1.5_f64.powi(2))).exp();
        weight_sum += weight;
        weighted_second += weight * squared;
    }
    let variance = 2.0 * 100.0 * weighted_second / weight_sum;
    let (c1, c2) = ((0.01_f64 * 255.0).powi(2), (0.03_f64 * 255.0).powi(2));
    let expected = ((2.0 * 100.0 * 155.0 + c1) * (c2 - 2.0 * variance))
        / ((100.0_f64.powi(2) + 155.0_f64.powi(2) + c1) * (2.0 * variance + c2));
    let inverted = measure(&ramp, &inverse).unwrap();
    assert!(inverted.ssim_luma < 0.0);
    assert!((inverted.ssim_luma - expected).abs() < 1e-10);
    metrics::evidence(
        "analytic-metrics",
        serde_json::json!({"same": same,
        "black-white": opposite, "one-blue-code-value": single,
        "contrast-inverted-gaussian-ramp":inverted,
        "physical_codec": false, "result": "passed"}),
    );
}

#[test]
fn batches_task_b6_quality_binding_rejects_foreign_capture_and_missing_geometry() {
    let source = capture(FrameSize::new(64, 64), 1);
    let identity = DecodedFrame {
        frame_id: 1,
        capture_ts_us: 33_333,
        keyframe: true,
    };
    for foreign in [
        DecodedFrame {
            frame_id: 2,
            ..identity
        },
        DecodedFrame {
            capture_ts_us: 44_444,
            ..identity
        },
        DecodedFrame {
            keyframe: false,
            ..identity
        },
    ] {
        assert!(
            metrics::paired(&source, identity, (&source, identity), (&source, foreign)).is_err()
        );
        assert!(
            metrics::paired(&source, identity, (&source, foreign), (&source, identity)).is_err()
        );
    }
    assert!(measure(&source, &constant(0)).is_err());
    assert!(measure(
        &BgraImage::new(FrameSize::new(8, 8)).unwrap(),
        &BgraImage::new(FrameSize::new(8, 8)).unwrap()
    )
    .is_err());
    let mut alpha = source.clone();
    alpha.data_mut()[3] = 0;
    let quality = measure(&source, &alpha).unwrap();
    assert_eq!(quality.different_pixels, 1);
    assert_eq!(quality.different_alpha_pixels, 1);
    assert!(quality.psnr_infinite);
    assert_eq!(quality.ssim_luma, 1.0);
}
