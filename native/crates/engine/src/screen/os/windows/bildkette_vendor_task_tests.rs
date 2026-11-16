//! Task tests of package BK12 (plan 2026-11-14 bildkette, B-22, B-21, B-36):
//! loaders without the libraries, rank before Media Foundation, request
//! checks, settings of NVENC and AMF, rate parameters, reference recovery,
//! registration table and the FFI layouts. The two tests that encode need
//! the vendor's GPU and stay ignored in an ordinary `cargo test`.
use std::io;
use std::mem::{offset_of, size_of};
use std::sync::Arc;

use super::amf_abi::{self as amf, prop, AmfVariant};
use super::amf_codecs::AVC;
use super::amf_settings::{dynamic_settings, rate_settings, static_settings, LTR_FRAMES};
use super::nvenc_device::{evictions, REGISTERED_IDLE, REGISTERED_SLOTS};
use super::nvenc_encode::{apply_reference_settings, apply_settings, RFI_DPB};
use super::nvenc_ffi::{self as nv, Zeroed};
use super::vendor_encode::{avc_level_for, LtrPlan, LtrSlots, RateParams, RefHistory};
use super::*;
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::host_backend::EncodedFrame;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::BgraImage;

const H264: CodecKind = CodecKind::H264;

fn choice(profile: H264Profile) -> CodecChoice {
    CodecChoice {
        codec: CodecKind::H264,
        profile: Some(profile),
        encoder_hardware: true,
        decoder_hardware: true,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}

/// A stand-in for a Media Foundation transform.
struct FakeMf(&'static str);

struct FakeEncoder(FrameSize);

impl ScreenEncoder for FakeEncoder {
    fn name(&self) -> String {
        "fake".into()
    }
    fn size(&self) -> FrameSize {
        self.0
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("not used"))
    }
}

impl EncoderFactory for FakeMf {
    fn name(&self) -> String {
        self.0.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::h264(H264Profile::High, true)]
    }
    fn open(&self, _: &CodecChoice, size: FrameSize, _: u32) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(Box::new(FakeEncoder(size)))
    }
}

fn media_foundation() -> Vec<Arc<dyn EncoderFactory>> {
    vec![
        Arc::new(FakeMf("mf-hardware")),
        Arc::new(FakeMf("mf-software")),
    ]
}

fn names(list: &[Arc<dyn EncoderFactory>]) -> Vec<String> {
    list.iter().map(|factory| factory.name()).collect()
}

#[test]
fn bildkette_task_win_vendor_loaders_report_absent() {
    let missing = nv::load_from("se-bk12-missing-nvenc.dll").err().unwrap();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    let missing = amf_encode::loads("se-bk12-missing-amf.dll").unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    // A driver older than API 12.1 counts as absent.
    assert!(nv::supported(0xc1) && nv::supported(0xd0) && !nv::supported(0xc0));
    // No library, no row: nothing changes on a machine without the GPU.
    let found = found();
    if nv::load().is_err() {
        assert_eq!(found.nvenc, [None; 3]);
    }
    if !amf_encode::present() {
        assert_eq!(found.amf, [None; 3]);
    }
    let rank = vendor_encoders(media_foundation());
    let vendors = found.nvenc.iter().chain(&found.amf).flatten().count();
    assert_eq!(names(&rank[vendors..]), ["mf-hardware", "mf-software"]);
}

#[test]
fn bildkette_task_win_vendor_rank_before_media_foundation() {
    let max = FrameSize::new(4096, 4096);
    let both = Found {
        nvenc: [Some(max), None, None],
        amf: [Some(FrameSize::new(3840, 2160)), None, None],
    };
    let rank = ranked(both, media_foundation());
    assert_eq!(names(&rank), ["nvenc", "amf", "mf-hardware", "mf-software"]);
    let offer = rank[0].offers();
    assert_eq!(
        offer,
        vec![CodecOffer::h264(H264Profile::High, true).with_max(max)]
    );
    assert!(rank[1].offers()[0].hardware);
    // Without a vendor interface the rank is Media Foundation's, unchanged.
    let none = ranked(Found::default(), media_foundation());
    assert_eq!(names(&none), ["mf-hardware", "mf-software"]);
    // A vendor encoder that cannot open gives way to the next stage: here
    // the non-admitted open, which a vendor encoder always refuses.
    let registry = CodecRegistry::new(rank, Vec::new());
    let size = FrameSize::new(1920, 1080);
    let opened = registry
        .open_encoder(&choice(H264Profile::High), size, 60)
        .unwrap();
    assert_eq!(opened.factory, "mf-hardware");
    // Refused (failed while encoding) vendor encoders are skipped, too.
    let refused = ["nvenc".to_string(), "amf".to_string()];
    let opened = registry
        .open_encoder_except(&choice(H264Profile::High), size, 60, &refused)
        .unwrap();
    assert_eq!(opened.factory, "mf-hardware");
    let report = registry.report(crate::screen::core::codec_registry::OpenH264Report::new(
        crate::screen::core::codec_registry::OpenH264State::Absent,
        None,
    ));
    let rows: Vec<(&str, bool)> = report
        .encoders
        .iter()
        .map(|row| (row.name.as_str(), row.hardware))
        .collect();
    assert_eq!(rows[..2], [("nvenc", true), ("amf", true)]);
}

#[test]
fn bildkette_task_win_vendor_request_check() {
    let max = FrameSize::new(4096, 4096);
    let high = choice(H264Profile::High);
    assert!(check_request(H264, max, &high, FrameSize::new(2560, 1440)).is_ok());
    let baseline = choice(H264Profile::ConstrainedBaseline);
    let refused = check_request(H264, max, &baseline, FrameSize::new(1920, 1080));
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::Unsupported);
    for size in [
        FrameSize::new(1921, 1080),
        FrameSize::new(8, 8),
        FrameSize::new(4098, 1080),
    ] {
        assert!(check_request(H264, max, &high, size).is_err(), "{size:?}");
    }
    let small = FrameSize::new(1920, 1080);
    assert!(check_request(H264, small, &high, FrameSize::new(2560, 1440)).is_err());
    let mut narrow = high;
    narrow.max_pixels = 1280 * 720;
    assert!(check_request(H264, max, &narrow, FrameSize::new(1920, 1080)).is_err());
}

#[test]
fn bildkette_task_win_vendor_rate_params() {
    let rate = RateParams::for_target(30_000_000, 120);
    assert_eq!(rate.bits_per_second, 30_000_000);
    assert_eq!(
        (rate.buffer_bits, rate.initial_fullness),
        (250_000, 250_000)
    );
    assert_eq!(rate.fps, 120);
    let floor = RateParams::for_target(1, 0);
    assert_eq!(floor.fps, 1);
    assert!(u64::from(floor.bits_per_second) >= MIN_TEST_BPS);
    assert!(!rate.differs(&RateParams::for_target(30_500_000, 120)));
    assert!(rate.differs(&RateParams::for_target(31_000_000, 120)));
    assert!(
        rate.differs(&RateParams::for_target(30_000_000, 144)),
        "fps"
    );
    assert_eq!(avc_level_for(FrameSize::new(1920, 1080), 60), 42);
    assert_eq!(avc_level_for(FrameSize::new(3840, 2160), 60), 52);
}

const MIN_TEST_BPS: u64 = crate::screen::core::rate_control::MIN_BITRATE_BPS;

#[test]
fn bildkette_task_win_vendor_nvenc_settings() {
    let mut config = nv::NvEncConfig::zeroed();
    config.rc_params.flags = nv::RC_ENABLE_LOOKAHEAD; // a preset with look-ahead
    config.h264.max_num_ref_frames = 2;
    let rate = RateParams::for_target(30_000_000, 120);
    apply_settings(&mut config, &rate);
    assert_eq!(config.gop_length, nv::NVENC_INFINITE_GOPLENGTH);
    assert_eq!(config.frame_interval_p, 1, "no B pictures");
    assert_eq!(config.profile_guid, nv::NV_ENC_H264_PROFILE_HIGH_GUID);
    let rc = &config.rc_params;
    assert_eq!(rc.rate_control_mode, nv::NV_ENC_PARAMS_RC_CBR);
    assert_eq!(
        (rc.average_bit_rate, rc.max_bit_rate),
        (30_000_000, 30_000_000)
    );
    assert_eq!(
        (rc.vbv_buffer_size, rc.vbv_initial_delay),
        (250_000, 250_000)
    );
    assert_eq!(rc.flags, nv::RC_ZERO_REORDER_DELAY);
    assert_eq!(
        config.h264.flags & nv::H264_REPEAT_SPSPPS,
        nv::H264_REPEAT_SPSPPS
    );
    assert_eq!(config.h264.idr_period, nv::NVENC_INFINITE_GOPLENGTH);
    assert_eq!(config.h264.vui.colour_matrix, nv::VUI_BT709);
    assert_eq!(config.h264.vui.video_full_range_flag, 0);
    let mut without = config;
    apply_reference_settings(&mut without, false);
    assert_eq!(without.rc_params.low_delay_key_frame_scale, 1);
    assert_eq!(without.h264.max_num_ref_frames, 2, "preset value kept");
    apply_reference_settings(&mut config, true);
    assert_eq!(config.h264.max_num_ref_frames, RFI_DPB);
    assert_eq!(config.h264.num_ref_l0, nv::NV_ENC_NUM_REF_FRAMES_1);
}

#[test]
fn bildkette_task_win_vendor_amf_settings() {
    let size = FrameSize::new(2560, 1440);
    let rate = RateParams::for_target(40_000_000, 100);
    let settings = static_settings(&AVC, size, &rate);
    let value = |list: &[(&str, AmfVariant)], name: &str| {
        list.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    assert_eq!(settings[0].0, prop::USAGE, "Usage first");
    let int = |name| value(&settings, name).as_int64().unwrap();
    assert_eq!(int(prop::USAGE), prop::USAGE_ULTRA_LOW_LATENCY);
    assert_eq!(int(prop::PROFILE), prop::PROFILE_HIGH);
    assert_eq!(
        int(prop::PROFILE_LEVEL),
        i64::from(avc_level_for(size, 100))
    );
    assert_eq!(int(prop::QUALITY_PRESET), prop::QUALITY_SPEED);
    assert_eq!(int(prop::RATE_CONTROL_METHOD), prop::RATE_CONTROL_CBR);
    assert_eq!(int(prop::B_PIC_PATTERN), 0, "no B pictures");
    assert_eq!(int(prop::MAX_LTR_FRAMES), LTR_FRAMES);
    assert_eq!(int(prop::OUTPUT_COLOR_PROFILE), prop::COLOR_PROFILE_709);
    assert_eq!(
        value(&settings, prop::LOWLATENCY_MODE).as_bool(),
        Some(true)
    );
    assert_eq!(
        value(&settings, prop::FULL_RANGE_COLOR).as_bool(),
        Some(false)
    );
    let frame = value(&settings, prop::FRAMESIZE).as_size().unwrap();
    assert_eq!((frame.width, frame.height), (2560, 1440));
    let rates = rate_settings(&AVC, &rate);
    let rint = |name| value(&rates, name).as_int64().unwrap();
    assert_eq!(rint(prop::TARGET_BITRATE), 40_000_000);
    assert_eq!(rint(prop::PEAK_BITRATE), 40_000_000, "CBR");
    assert_eq!(rint(prop::VBV_BUFFER_SIZE), 400_000, "one frame");
    assert_eq!(rint(prop::INITIAL_VBV_FULLNESS), prop::VBV_FULL);
    let fps = value(&rates, prop::FRAMERATE).as_rate().unwrap();
    assert_eq!((fps.num, fps.den), (100, 1));
    assert!(rates
        .iter()
        .all(|(name, _)| settings.iter().any(|(key, _)| key == name)));
    let after = dynamic_settings(&AVC);
    assert_eq!(value(&after, prop::IDR_PERIOD).as_int64(), Some(0));
    assert_eq!(value(&after, prop::ENFORCE_HRD).as_bool(), Some(true));
    assert_eq!(
        value(&after, prop::SKIP_FRAME_ENABLE).as_bool(),
        Some(false)
    );
    assert_eq!(
        value(&after, prop::FILLER_DATA_ENABLE).as_bool(),
        Some(false)
    );
}

#[test]
fn bildkette_task_win_vendor_reference_recovery() {
    // NVENC: invalidation only while a confirmed picture is in the DPB.
    let mut history = RefHistory::new(RFI_DPB);
    history.encoded(10, true);
    for frame in [11, 12, 13] {
        history.encoded(frame, false);
    }
    assert_eq!(history.invalidate_after(11), Some(vec![12, 13]));
    assert_eq!(history.invalidate_after(11), Some(Vec::new()));
    assert_eq!(history.invalidate_after(9), None, "older than the IDR");
    history.encoded(20, true);
    assert_eq!(history.invalidate_after(14), None, "IDR empties the DPB");
    let mut full = RefHistory::new(RFI_DPB);
    full.encoded(30, true);
    for frame in 31..=36 {
        full.encoded(frame, false);
    }
    assert_eq!(full.invalidate_after(31), None, "range as large as the DPB");
    assert_eq!(full.invalidate_after(33), Some(vec![34, 35, 36]));
    // AMF: two LTRs, marked every quarter second and on every IDR.
    let mut ltr = LtrSlots::new(60);
    let mark = |slot| LtrPlan {
        mark: Some(slot),
        force: None,
    };
    assert_eq!(ltr.plan(100, true), mark(0));
    for frame in 101..115 {
        assert_eq!(ltr.plan(frame, false), LtrPlan::default(), "{frame}");
    }
    assert_eq!(ltr.plan(115, false), mark(1));
    for frame in 116..130 {
        ltr.plan(frame, false);
    }
    assert_eq!(ltr.plan(130, false), mark(0));
    assert!(ltr.recover(120), "frame 115 is confirmed");
    assert_eq!(ltr.plan(131, false).force, Some(1 << 1));
    assert!(!ltr.recover(99), "no marked picture that old");
    assert_eq!(ltr.plan(132, false).force, None);
    assert_eq!(ltr.plan(133, true), mark(0), "IDR restarts the slots");
}

#[test]
fn bildkette_task_win_vendor_registration_table() {
    assert!(evictions(&[1, 2, 3], 4).is_empty());
    assert_eq!(evictions(&[4, 1, 3, 2], 5), vec![1], "least recently used");
    let idle = 1 + REGISTERED_IDLE + 1;
    assert_eq!(evictions(&[1, idle], idle + 1), vec![0], "idle ring");
    assert_eq!(REGISTERED_SLOTS, 4);
}

#[test]
fn bildkette_task_win_vendor_nvenc_struct_sizes() {
    assert_eq!(size_of::<nv::Guid>(), 16);
    assert_eq!(size_of::<nv::NvEncRcParams>(), 128);
    assert_eq!(offset_of!(nv::NvEncRcParams, low_delay_key_frame_scale), 92);
    assert_eq!(size_of::<nv::NvEncVui>(), 112);
    assert_eq!(size_of::<nv::NvEncConfigH264>(), 1792);
    assert_eq!(offset_of!(nv::NvEncConfigH264, max_num_ref_frames), 60);
    assert_eq!(offset_of!(nv::NvEncConfigH264, num_ref_l0), 204);
    assert_eq!(size_of::<nv::NvEncConfig>(), 3584);
    assert_eq!(offset_of!(nv::NvEncConfig, rc_params), 40);
    assert_eq!(offset_of!(nv::NvEncConfig, h264), 168);
    assert_eq!(size_of::<nv::NvEncPresetConfig>(), 5128);
    assert_eq!(size_of::<nv::NvEncInitializeParams>(), 1808);
    assert_eq!(offset_of!(nv::NvEncInitializeParams, encode_config), 88);
    assert_eq!(offset_of!(nv::NvEncInitializeParams, tuning_info), 136);
    assert_eq!(size_of::<nv::NvEncReconfigureParams>(), 1824);
    assert_eq!(size_of::<nv::NvEncPicParams>(), 3360);
    assert_eq!(offset_of!(nv::NvEncPicParams, input_time_stamp), 24);
    assert_eq!(offset_of!(nv::NvEncPicParams, codec_pic_params), 80);
    assert_eq!(size_of::<nv::NvEncLockBitstream>(), 1552);
    assert_eq!(offset_of!(nv::NvEncLockBitstream, bitstream_buffer_ptr), 56);
    assert_eq!(offset_of!(nv::NvEncLockBitstream, picture_type), 64);
    assert_eq!(size_of::<nv::NvEncCreateBitstreamBuffer>(), 776);
    assert_eq!(size_of::<nv::NvEncOpenSessionParams>(), 1552);
    assert_eq!(size_of::<nv::NvEncCapsParam>(), 256);
    assert_eq!(size_of::<nv::NvEncRegisterResource>(), 1536);
    assert_eq!(
        offset_of!(nv::NvEncRegisterResource, resource_to_register),
        24
    );
    assert_eq!(
        offset_of!(nv::NvEncRegisterResource, registered_resource),
        32
    );
    assert_eq!(offset_of!(nv::NvEncRegisterResource, buffer_format), 40);
    assert_eq!(size_of::<nv::NvEncMapInputResource>(), 1544);
    assert_eq!(offset_of!(nv::NvEncMapInputResource, mapped_resource), 24);
    assert_eq!(offset_of!(nv::NvEncMapInputResource, mapped_buffer_fmt), 32);
    assert_eq!(size_of::<nv::NvEncFunctions>(), 2552);
    assert_eq!(offset_of!(nv::NvEncFunctions, get_encode_caps), 64);
    assert_eq!(offset_of!(nv::NvEncFunctions, encode_picture), 136);
    assert_eq!(offset_of!(nv::NvEncFunctions, map_input_resource), 208);
    assert_eq!(offset_of!(nv::NvEncFunctions, unmap_input_resource), 216);
    assert_eq!(offset_of!(nv::NvEncFunctions, invalidate_ref_frames), 232);
    assert_eq!(offset_of!(nv::NvEncFunctions, open_encode_session_ex), 240);
    assert_eq!(offset_of!(nv::NvEncFunctions, register_resource), 248);
    assert_eq!(offset_of!(nv::NvEncFunctions, reconfigure_encoder), 264);
    assert_eq!(
        offset_of!(nv::NvEncFunctions, get_encode_preset_config_ex),
        320
    );
    assert_eq!(nv::NV_ENC_CONFIG_VER, 0xf108_000c);
    assert_eq!(nv::NV_ENC_PIC_PARAMS_VER, 0xf106_000c);
    assert_eq!(nv::NV_ENCODE_API_FUNCTION_LIST_VER, 0x7102_000c);
    assert_eq!(nv::NV_ENC_REGISTER_RESOURCE_VER, 0x7104_000c);
    assert_eq!(nv::NV_ENC_MAP_INPUT_RESOURCE_VER, 0x7104_000c);
}

#[test]
fn bildkette_task_win_vendor_amf_layouts() {
    assert_eq!(size_of::<amf::AmfVariant>(), 24);
    assert_eq!(offset_of!(amf::AmfVariant, value), 8);
    assert_eq!(size_of::<amf::AmfGuid>(), 16);
    assert_eq!(size_of::<amf::InterfaceVtbl>(), 24);
    assert_eq!(size_of::<amf::StorageVtbl>(), 80);
    assert_eq!(size_of::<amf::DataVtbl>(), 184);
    assert_eq!(offset_of!(amf::DataVtbl, set_pts), 152);
    assert_eq!(size_of::<amf::BufferVtbl>(), 224);
    assert_eq!(offset_of!(amf::BufferVtbl, get_size), 192);
    assert_eq!(offset_of!(amf::BufferVtbl, get_native), 200);
    assert_eq!(size_of::<amf::ContextVtbl>(), 440);
    assert_eq!(offset_of!(amf::ContextVtbl, terminate), 104);
    assert_eq!(offset_of!(amf::ContextVtbl, init_dx11), 144);
    assert_eq!(
        offset_of!(amf::ContextVtbl, create_surface_from_dx11_native),
        392
    );
    assert_eq!(size_of::<amf::ComponentVtbl>(), 224);
    assert_eq!(offset_of!(amf::ComponentVtbl, init), 136);
    assert_eq!(offset_of!(amf::ComponentVtbl, terminate), 152);
    assert_eq!(offset_of!(amf::ComponentVtbl, submit_input), 176);
    assert_eq!(offset_of!(amf::ComponentVtbl, query_output), 184);
    assert_eq!(offset_of!(amf::ComponentVtbl, get_caps), 208);
    assert_eq!(size_of::<amf::FactoryVtbl>(), 56);
    assert_eq!(offset_of!(amf::FactoryVtbl, create_component), 8);
    assert_eq!(size_of::<amf::CapsVtbl>(), 128);
    assert_eq!(offset_of!(amf::CapsVtbl, get_input_caps), 112);
    assert_eq!(size_of::<amf::IoCapsVtbl>(), 88);
    assert_eq!(offset_of!(amf::IoCapsVtbl, get_height_range), 32);
    assert_eq!(amf::AMF_FULL_VERSION, 0x0001_0004_0023_0000);
    assert_eq!(amf::wide("Usage"), [85, 115, 97, 103, 101, 0]);
    let rate = AmfVariant::rate(60, 1);
    assert_eq!(rate.kind, amf::VARIANT_RATE);
    assert_eq!(rate.as_int64(), None, "tagged");
}

/// Encodes three pictures of an NV12 texture with `session`, then asks for
/// a reference to the first after a reset.
fn encode_three(session: &mut dyn vendor_encode::VendorSession, device: &ID3D11Device) {
    use super::vendor_encode::VendorPicture as Picture;
    use crate::screen::os::mf_encoder::video_convert::nv12_texture;
    let size = FrameSize::new(1280, 720);
    let texture = nv12_texture(device, size, None).unwrap();
    let mut rate = RateParams::for_target(8_000_000, 60);
    let mut keys = Vec::new();
    for frame in 1..=3u64 {
        let mut unit = Vec::new();
        let picture = Picture {
            texture: &texture,
            frame_id: frame,
            idr: frame == 1,
        };
        let key = session
            .encode(picture, &rate, &mut |bytes| {
                unit.extend_from_slice(bytes);
                Ok(())
            })
            .unwrap();
        crate::screen::core::frame_size_caps::check_avc_dimensions(
            &unit,
            size,
            FrameLimits::RESOURCE,
        )
        .unwrap();
        keys.push(key);
        // A new bit rate and frame rate without a restart.
        rate = RateParams::for_target(12_000_000, 120);
    }
    assert_eq!(keys, [true, false, false]);
    let recovery = session.recover_references(1);
    let next = Picture {
        texture: &texture,
        frame_id: 4,
        idr: recovery == vendor_encode::Recovery::KeyframeNeeded,
    };
    let key = session.encode(next, &rate, &mut |_| Ok(())).unwrap();
    assert_eq!(key, recovery == vendor_encode::Recovery::KeyframeNeeded);
}

#[test]
#[ignore = "needs an NVIDIA GPU with driver branch 530 or later; Suite keine (kein Runner mit Grafikkarte)"]
fn bildkette_task_win_vendor_nvenc_encodes_on_the_gpu() {
    let device = adapter_device(VENDOR_NVIDIA).expect("an NVIDIA adapter");
    assert!(found().nvenc[0].is_some(), "probe finds NVENC");
    let size = FrameSize::new(1280, 720);
    let mut session = nvenc_encode::NvencSession::open(&device, H264, size, 60).unwrap();
    encode_three(&mut session, &device);
}

#[test]
#[ignore = "needs an AMD GPU with the AMF runtime amfrt64.dll; Suite keine (kein Runner mit Grafikkarte)"]
fn bildkette_task_win_vendor_amf_encodes_on_the_gpu() {
    let device = adapter_device(VENDOR_AMD).expect("an AMD adapter");
    assert!(found().amf[0].is_some(), "probe finds AMF");
    let size = FrameSize::new(1280, 720);
    let mut session = amf_encode::AmfSession::open(&device, H264, size, 60).unwrap();
    encode_three(&mut session, &device);
}
