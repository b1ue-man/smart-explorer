//! Task tests of merge W1 (plan 2026-11-14 „Bildkette“, requests 3 and 7):
//! the Media Foundation encoders of HEVC and AV1 are connected, refuse
//! 4:4:4, and a hybrid laptop's capture adapter has its own MFT first; the
//! adapter blob of `MFT_ENUM_ADAPTER_LUID`. Without Media Foundation (the
//! GitHub runner) the part that needs a media type returns early.
use super::super::{EncoderFactory, MfEncoderFactory, MF_VIDEO_ENCODERS_CONNECTED};
use super::*;
use crate::screen::core::negotiate::CodecChoice;

fn factory(kind: Kind, codec: CodecKind, adapter: Option<u64>) -> MfEncoderFactory {
    MfEncoderFactory {
        kind,
        codec,
        index: 0,
        name: "mft-test".into(),
        adapter,
        caps: OnceLock::new(),
        refused: OnceLock::new(),
    }
}

fn choice(codec: CodecKind, profile: Option<H264Profile>) -> CodecChoice {
    CodecChoice {
        codec,
        profile,
        encoder_hardware: true,
        decoder_hardware: true,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}

#[test]
fn bildkette_task_win_w1_mf_hevc_av1_encoders_are_connected() {
    assert!(MF_VIDEO_ENCODERS_CONNECTED);
    for codec in [CodecKind::Hevc, CodecKind::Av1] {
        let hardware = factory(Kind::HardwareEncoder, codec, None);
        assert_eq!(
            hardware.offers(),
            vec![CodecOffer::video(codec, false, true)],
            "{codec:?}: Main 4:2:0 in hardware"
        );
        // 4:4:4 (`profile` High of HEVC/AV1) is refused before any MFT.
        let sharp = choice(codec, Some(H264Profile::High));
        let refused = hardware.open(&sharp, FrameSize::new(1920, 1080), 60);
        assert_eq!(
            refused.err().map(|e| e.kind()),
            Some(io::ErrorKind::Unsupported)
        );
        // Another codec than the factory's is refused as before.
        let other = choice(CodecKind::H264, Some(H264Profile::High));
        let refused = hardware.open(&other, FrameSize::new(1920, 1080), 60);
        assert_eq!(
            refused.err().map(|e| e.kind()),
            Some(io::ErrorKind::Unsupported)
        );
    }
    // A software MFT has no hardware settings to refuse.
    let software = factory(Kind::SoftwareEncoder, CodecKind::H264, None);
    assert!(software.refused_settings().is_empty());
}

#[test]
fn bildkette_task_win_w1_mf_output_type_names_the_codec() {
    if platform().is_err() {
        return;
    }
    use crate::screen::os::mf_common::EncoderSetup;
    use windows::Win32::Media::MediaFoundation::{MF_MT_MPEG2_PROFILE, MF_MT_SUBTYPE};
    for (codec, subtype) in [
        (CodecKind::H264, MFVideoFormat_H264),
        (CodecKind::Hevc, MFVideoFormat_HEVC),
        (CodecKind::Av1, MFVideoFormat_AV1),
    ] {
        let setup = EncoderSetup {
            name: "mft-test".into(),
            profile: H264Profile::High,
            size: FrameSize::new(1280, 720),
            fps: 60,
            codec,
            adapter: None,
        };
        let media = crate::screen::os::mf_encoder_sync::output_type(&setup, 8_000_000)
            .expect("output type");
        // SAFETY: attribute reads on an owned media type.
        let (written, profile) = unsafe {
            (
                media.GetGUID(&MF_MT_SUBTYPE).expect("subtype"),
                media.GetUINT32(&MF_MT_MPEG2_PROFILE).expect("profile"),
            )
        };
        assert_eq!(written, subtype, "{codec:?}");
        // H.264 High is 100 (`eAVEncH264VProfile_High`); HEVC and AV1 Main
        // 8-bit 4:2:0 are 1 in their enumerations.
        let expected = if codec == CodecKind::H264 { 100 } else { 1 };
        assert_eq!(profile, expected, "{codec:?}");
    }
}

#[test]
fn bildkette_task_win_w1_mf_capture_adapter_first() {
    let (igpu, dgpu) = (0x0000_0000_0000_a001_u64, 0x0000_0001_0000_b002_u64);
    let entries = [
        Entry {
            name: "Intel Quick Sync".into(),
            adapter: Some(igpu),
        },
        Entry {
            name: "NVIDIA MFT".into(),
            adapter: Some(dgpu),
        },
    ];
    // The display (and the capture) hangs on the iGPU: the dGPU's MFT
    // gives way, the iGPU's opens.
    assert!(prefer_capture_adapter(Some(dgpu), igpu, &entries));
    assert!(!prefer_capture_adapter(Some(igpu), igpu, &entries));
    // An MFT of unknown adapter, or no MFT on the capture's adapter: as
    // before (the texture route decides, else system memory).
    assert!(!prefer_capture_adapter(None, igpu, &entries));
    assert!(!prefer_capture_adapter(Some(dgpu), igpu, &entries[1..]));
    // The adapter blob: `LowPart`, then `HighPart`, as `DeviceSet` packs it.
    for packed in [igpu, dgpu, u64::MAX, 0] {
        assert_eq!(luid_of(luid_bytes(packed)), packed);
    }
    let bytes = luid_bytes(dgpu);
    assert_eq!(&bytes[..4], &0xb002u32.to_ne_bytes());
    assert_eq!(&bytes[4..], &1u32.to_ne_bytes());
}

#[test]
fn bildkette_task_win_w1_video_codecs_share_the_pipeline_rules() {
    // BK6 hand-over to BK3 (merge W1): the smallest picture and the rate of
    // the pairing hold for every video codec, not only H.264.
    let produce = include_str!("../../core/host_pipeline_produce.rs");
    assert!(produce.contains("let video = self.choice.codec.is_video();"));
    assert!(produce.contains("if video && (fitted.width < 2 || fitted.height < 2)"));
    assert!(produce.contains("let rate = if video {"));
    assert!(!produce.contains("codec == CodecKind::H264"));
    assert!(CodecKind::Hevc.is_video() && CodecKind::Av1.is_video());
    assert!(!CodecKind::Tiles.is_video());
}
