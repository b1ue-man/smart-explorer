//! Task tests W4 (plan 2026-11-14 „Bildkette“, review B2 and R1):
//! GPU-to-system-memory MFT scaling within two `CodecInput` claims plus
//! `SyncEncoder` refresh bookkeeping, and R1's AMF capability names and
//! session bit-rate ceiling.
use std::io;
use std::sync::Arc;

use super::amf_encode::caps::{ceiling_of, max_bitrate_cap};
use super::vendor_encode::{avc_level_for, RateParams};
use super::*;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureOwner, CapturePlan, SourceKey,
};
use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract, MAX_WORKING_BYTES};
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::core::host_codec_adapters::{
    HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan,
};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{avc_level_max_bitrate_bps, MIN_BITRATE_BPS};
use crate::screen::os::mf_capture_media::stage_claims;
use crate::screen::os::mf_encoder::video_convert::nv12_len;
use crate::screen::os::mf_encoder_sync::update_refresh;

#[test]
fn bildkette_task_win_w4_scaled_gpu_input_stays_within_two_codec_input_claims() {
    let (size, ring_size) = (FrameSize::new(1920, 1080), FrameSize::new(3840, 2160));
    let caps = FrameCapabilities::admitted(size, 32 * 1024 * 1024).unwrap();
    let contract = FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap();
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "w4-mf-claims",
        )
        .unwrap();
    admission.bind_contract(Some(contract)).unwrap();
    let mut plan = CapturePlan::new(
        SourceKey::new("w4-mf-claims", "gpu").unwrap(),
        CaptureGeneration::new(1),
    );
    plan.bind_contract(Some(contract)).unwrap();
    HostOutputPlan::new(
        HostContract::Versioned(contract),
        size,
        HostCodecPlan::MediaFoundation {
            output_buffer_bytes: size.bgra_len() as u64,
        },
    )
    .unwrap()
    .for_worker()
    .append_to(&mut plan, HostInputPixels::BorrowedSource)
    .unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let yuv = nv12_len(size);

    let ((), staging) = stage_claims(
        &grant,
        ring_size,
        size,
        |_| panic!("ring_size != size must take the scaled path"),
        |staging_slice, scaled_slice| {
            assert_eq!((staging_slice.bytes(), scaled_slice.bytes()), (yuv, yuv));
            assert_eq!(grant.live_bytes().unwrap(), 2 * yuv);
            let probe = grant.claim(CaptureOwner::CodecInput, yuv).unwrap();
            let third = grant.claim(CaptureOwner::CodecInput, yuv);
            assert_eq!(
                third.err().map(|error| error.kind()),
                Some(io::ErrorKind::WouldBlock)
            );
            drop(probe);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(grant.live_bytes().unwrap(), yuv);
    let mf_buffer = grant.claim(CaptureOwner::CodecInput, yuv).unwrap();
    assert_eq!(grant.live_bytes().unwrap(), 2 * yuv);
    drop(staging);
    assert_eq!(grant.live_bytes().unwrap(), yuv);

    let ((), next_staging) =
        stage_claims(&grant, ring_size, size, |_| Ok(()), |_, _| Ok(())).unwrap();
    drop(next_staging);
    drop(mf_buffer);
    assert_eq!(grant.live_bytes().unwrap(), 0);

    let mut refresh = false;
    update_refresh::<()>(
        &mut refresh,
        &Err(io::Error::from(io::ErrorKind::WouldBlock)),
    );
    assert!(refresh);
    update_refresh::<()>(&mut refresh, &Err(io::Error::other("other")));
    assert!(refresh);
    update_refresh(&mut refresh, &Ok(()));
    assert!(!refresh);
}

#[test]
fn bildkette_task_win_w4_amf_caps_and_vendor_rate_ceiling() {
    assert_eq!(max_bitrate_cap(CodecKind::H264), Some("MaxBitrate"));
    assert_eq!(max_bitrate_cap(CodecKind::Hevc), Some("HevcMaxBitrate"));
    assert_eq!(max_bitrate_cap(CodecKind::Av1), Some("Av1MaxBitrate"));
    assert_eq!(max_bitrate_cap(CodecKind::Tiles), None);
    let level = avc_level_for(FrameSize::new(1920, 1080), 60);
    let level_bps = avc_level_max_bitrate_bps(u32::from(level), H264Profile::High).unwrap();
    assert_eq!(
        ceiling_of(Some(level_bps), Some(100_000_000)),
        Some(level_bps)
    );
    assert_eq!(
        ceiling_of(Some(level_bps), Some(25_000_000)),
        Some(25_000_000)
    );
    assert_eq!(ceiling_of(None, Some(MIN_BITRATE_BPS - 1)), None);
    assert_eq!(ceiling_of(None, None), None);
}

#[test]
#[ignore = "needs an NVIDIA GPU with driver branch 530 or later; Suite keine (kein Runner mit Grafikkarte)"]
fn bildkette_task_win_w4_nvenc_async_encodes_on_gpu() {
    let device = adapter_device(VENDOR_NVIDIA).expect("an NVIDIA adapter");
    let size = FrameSize::new(1280, 720);
    let mut session = nvenc_encode::NvencSession::open(&device, CodecKind::H264, size, 60).unwrap();
    let texture =
        crate::screen::os::mf_encoder::video_convert::nv12_texture(&device, size, None).unwrap();
    let rate = RateParams::for_target(8_000_000, 60);
    let mut unit = Vec::new();
    let key = vendor_encode::VendorSession::encode(
        &mut session,
        vendor_encode::VendorPicture {
            texture: &texture,
            frame_id: 1,
            idr: true,
        },
        &rate,
        &mut |bytes| {
            unit.extend_from_slice(bytes);
            Ok(())
        },
    )
    .unwrap();
    assert!(key && !unit.is_empty());
}
