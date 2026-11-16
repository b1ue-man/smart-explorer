//! BK4 task tests (`bildkette_task_linux_hw_*`): the Linux hardware codecs
//! without hardware. Loaders report "absent", the order of plan 4.10a, the
//! B-21 parameter mapping, the stream layout and the C structure sizes and
//! offsets counted from libva 2.20.0 and NVENC API 12.1.
use std::io::ErrorKind;
use std::mem::{offset_of, size_of};
use std::path::PathBuf;
use std::sync::Arc;

use super::hw_encode_common::{avc_level_for, prefix_for, RateParams};
use super::nvenc_encode::{apply_settings, NvencEncoderFactory, NVENC_NAME};
use super::nvenc_ffi::{self as nv, Zeroed as _};
use super::vaapi_ffi::{self as va, VA_ATTRIB_NOT_SUPPORTED};
use super::vaapi_h264_abi as abi;
use super::vaapi_h264_headers::{
    misc_params, picture_params, pps, sequence_params, slice_params, sps, Layout, PictureState,
};
use super::vaapi_probe::{
    choose_encode, decode_profiles, max_picture, VaDecodeCaps, VaDevice, VaEncodeCaps,
};
use super::vaapi_vpp::{prime_descriptor, DmaBufFrame};
use crate::screen::core::codec_registry::{
    CodecRegistry, EncoderFactory, OpenH264Report, OpenH264State,
};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::negotiate::{
    negotiate, CodecKind, CodecOffer, CodecPreference, H264Profile,
};
use crate::screen::core::pixel_convert::FrameSize;

fn device() -> Arc<VaDevice> {
    Arc::new(VaDevice {
        node: PathBuf::from("/nonexistent-bk4/renderD128"),
        encode: vec![VaEncodeCaps {
            profile: H264Profile::High,
            va_profile: va::VA_PROFILE_H264_HIGH,
            entrypoint: va::VA_ENTRYPOINT_ENC_SLICE_LP,
            packed_headers: 3,
            max: FrameSize::new(4096, 2304),
        }],
        decode: Some(VaDecodeCaps {
            va_profile: va::VA_PROFILE_H264_HIGH,
            profiles: vec![H264Profile::High, H264Profile::ConstrainedBaseline],
            max: FrameSize::new(4096, 2304),
        }),
        video_proc: true,
        // BK4b: no HEVC/AV1 here; `bildkette_task_linux_hw_hevc_*` and
        // `bildkette_task_linux_hw_av1_*` cover them.
        video_encode: Vec::new(),
        video_decode: Vec::new(),
    })
}

fn software_viewer() -> [CodecOffer; 1] {
    [CodecOffer::h264(H264Profile::High, false)]
}

#[test]
fn bildkette_task_linux_hw_loaders_report_absent() {
    let error = va::load_from("libva-bk4-absent.so.2", "libva-drm-bk4-absent.so.2")
        .err()
        .expect("a missing libva must not load");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    let (nvenc, cuda) = (
        "libnvidia-encode-bk4-absent.so.1",
        "libcuda-bk4-absent.so.1",
    );
    let error = nv::load_from(nvenc, cuda)
        .err()
        .expect("a missing NVENC must not load");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    let empty = tempfile::tempdir().unwrap();
    assert!(va::render_nodes(empty.path()).is_empty());
    for name in ["renderD129", "card0", "renderD128", "renderDx"] {
        std::fs::write(empty.path().join(name), b"").unwrap();
    }
    let nodes = va::render_nodes(empty.path());
    let names: Vec<_> = nodes.iter().map(|n| n.file_name().unwrap()).collect();
    assert_eq!(names, ["renderD128", "renderD129"]);
}

#[test]
fn bildkette_task_linux_hw_rank_and_report() {
    let nvenc = Some(FrameSize::new(4096, 4096));
    let (encoders, decoders) = super::ordered(nvenc, vec![device()]);
    let names: Vec<_> = encoders.iter().map(|f| f.name()).collect();
    assert_eq!(names, [NVENC_NAME, "vaapi-renderD128"]);
    assert_eq!(decoders.len(), 1);
    let registry = CodecRegistry::new(encoders, decoders);
    let report = registry.report(OpenH264Report::new(OpenH264State::Absent, None));
    let json = serde_json::to_value(&report).unwrap();
    for list in ["encoders", "decoders"] {
        let rows = json[list].as_array().unwrap();
        assert_eq!(rows.last().unwrap()["codec"], "tiles");
        for row in &rows[..rows.len() - 1] {
            assert_eq!(row["hardware"], true, "{row}");
            assert_eq!(row["codec"], "h264");
        }
    }
    // A software-only viewer gets the hardware encoder (tier hardware/software).
    let offers = registry.encoder_offers();
    let choices = negotiate(&offers, &software_viewer(), CodecPreference::Auto);
    assert!(choices[0].encoder_hardware);
    assert_eq!(choices.last().unwrap().codec, CodecKind::Tiles);
    // Opening tries the hardware factories in order: on a machine with
    // NVENC the first opens; without (CI) both refuse, NVENC first (the
    // VA device node does not exist).
    match registry.open_encoder(&choices[0], FrameSize::new(1280, 720), 60) {
        Ok(opened) => assert_eq!(opened.factory, NVENC_NAME),
        Err(error) => {
            let text = error.to_string();
            let vaapi = text.find("vaapi-renderD128").expect("VA-API was tried");
            let nvenc = text.find(NVENC_NAME);
            assert!(nvenc.is_some_and(|at| at < vaapi), "{text}");
        }
    }
}

#[test]
fn bildkette_task_linux_hw_without_devices_nothing_changes() {
    let (encoders, decoders) = super::ordered(None, Vec::new());
    assert!(encoders.is_empty() && decoders.is_empty());
    let small = NvencEncoderFactory {
        max: FrameSize::new(64, 64),
    };
    let offers = small.offers();
    let choice = negotiate(&offers, &software_viewer(), CodecPreference::H264)[0];
    let refused = small.open(&choice, FrameSize::new(1280, 720), 60);
    assert_eq!(refused.err().unwrap().kind(), ErrorKind::InvalidInput);
}

#[test]
fn bildkette_task_linux_hw_rate_params() {
    let rate = RateParams::for_target(6_000_000, 60);
    let mapped = (rate.bits_per_second, rate.buffer_bits, rate.fps);
    assert_eq!(mapped, (6_000_000, 100_000, 60));
    assert_eq!(rate.initial_fullness, rate.buffer_bits);
    assert_eq!(RateParams::for_target(1_000, 0).bits_per_second, 150_000);
    let highest = RateParams::for_target(u64::MAX, 30);
    // Merge W2: no fixed ceiling; the encoder bounds the rate by its level.
    assert_eq!(highest.bits_per_second, u32::MAX);
    assert!(!rate.differs(&RateParams::for_target(6_100_000, 60)));
    assert!(rate.differs(&RateParams::for_target(6_200_000, 60)));
    assert!(rate.differs(&RateParams::for_target(6_000_000, 120)));
    let (control, hrd, frame_rate) = misc_params(&rate, false);
    assert_eq!(control.bits_per_second, 6_000_000);
    assert_eq!(control.target_percentage, 100);
    assert_eq!(control.window_size, 16);
    assert_eq!(control.rc_flags, abi::RC_FLAG_DISABLE_FRAME_SKIP);
    let buffer = (hrd.buffer_size, hrd.initial_buffer_fullness);
    assert_eq!(buffer, (100_000, 100_000));
    assert_eq!(frame_rate.framerate, 60 | (1 << 16));
    let (control, _, _) = misc_params(&rate, true);
    assert_eq!(control.rc_flags & abi::RC_FLAG_RESET, abi::RC_FLAG_RESET);
}

#[test]
fn bildkette_task_linux_hw_stream_layout() {
    assert_eq!(avc_level_for(FrameSize::new(1920, 1080), 30), 40);
    assert_eq!(avc_level_for(FrameSize::new(1920, 1080), 60), 42);
    assert_eq!(avc_level_for(FrameSize::new(3840, 2160), 60), 52);
    let layout = Layout::new(FrameSize::new(1920, 1080));
    let geometry = (layout.width_mbs, layout.height_mbs, layout.crop());
    assert_eq!(geometry, (120, 68, (0, 4)));
    let rate = RateParams::for_target(20_000_000, 60);
    let seq = sequence_params(&layout, 42, &rate);
    let fixed = (seq.level_idc, seq.ip_period, seq.max_num_ref_frames);
    assert_eq!(fixed, (42, 1, 1));
    assert_eq!(seq.seq_fields & 3, 1, "4:2:0");
    assert_eq!((seq.seq_fields >> 6) & 0xf, 12, "frame_num of 16 bits");
    assert_eq!((seq.seq_fields >> 10) & 3, 2, "picture order count type 2");
    let cropping = (seq.frame_cropping_flag, seq.frame_crop_bottom_offset);
    assert_eq!(cropping, (1, 4));
    // The packed SPS passes the screen's own SPS check at this size.
    let header = sps(&layout, H264Profile::High, 42);
    crate::screen::core::frame_size_caps::check_avc_dimensions(
        &header,
        FrameSize::new(1920, 1080),
        FrameLimits::RESOURCE,
    )
    .unwrap();
    assert_eq!(&header[..5], &[0, 0, 0, 1, 0x67]);
    assert_eq!(header[5], 100, "High");
    let baseline = sps(&layout, H264Profile::ConstrainedBaseline, 42);
    assert_eq!(baseline[5..7], [66, 0xc0]);
    assert_eq!(&pps(H264Profile::High)[..5], &[0, 0, 0, 1, 0x68]);
}

#[test]
fn bildkette_task_linux_hw_picture_and_slice_params() {
    let layout = Layout::new(FrameSize::new(1280, 720));
    let idr = PictureState {
        idr: true,
        frame_num: 0,
        frames_since_idr: 0,
        idr_pic_id: 1,
        recon: 11,
        reference: Some(12),
        coded: 7,
    };
    let picture = picture_params(&idr, H264Profile::High);
    assert_eq!(picture.pic_fields & 1, 1, "idr_pic_flag");
    assert_eq!((picture.pic_fields >> 3) & 1, 1, "CABAC for High");
    assert_eq!(picture.reference_frames[0], abi::VaPictureH264::invalid());
    assert_eq!((picture.curr_pic.picture_id, picture.coded_buf), (11, 7));
    let slice = slice_params(&idr, &layout);
    assert_eq!((slice.slice_type, slice.num_macroblocks), (2, 80 * 45));
    let p = PictureState {
        idr: false,
        frame_num: 5,
        frames_since_idr: 5,
        ..idr
    };
    let picture = picture_params(&p, H264Profile::ConstrainedBaseline);
    assert_eq!(picture.pic_fields & 1, 0);
    assert_eq!((picture.pic_fields >> 3) & 1, 0, "CAVLC for CB");
    let reference = picture.reference_frames[0];
    assert_eq!((reference.picture_id, reference.frame_idx), (12, 4));
    assert_eq!(reference.flags, abi::VA_PICTURE_H264_SHORT_TERM_REFERENCE);
    let counts = (
        reference.top_field_order_cnt,
        picture.curr_pic.top_field_order_cnt,
    );
    assert_eq!(counts, (8, 10));
    let slice = slice_params(&p, &layout);
    assert_eq!(slice.slice_type, 0);
    assert_eq!(slice.ref_pic_list0[0], reference);
    assert_eq!(slice.ref_pic_list0[1], abi::VaPictureH264::invalid());
}

#[test]
fn bildkette_task_linux_hw_access_unit_prefix() {
    let layout = Layout::new(FrameSize::new(64, 64));
    let sets = [sps(&layout, H264Profile::High, 30), pps(H264Profile::High)].concat();
    let cached = Some(sets.as_slice());
    let idr_slice: &[u8] = &[0, 0, 0, 1, 0x65, 0x88, 0x80];
    let p_slice: &[u8] = &[0, 0, 0, 1, 0x41, 0x9a, 0x01];
    assert!(prefix_for(&[idr_slice], true, None).is_err());
    let prefix = prefix_for(&[idr_slice], true, cached).unwrap();
    assert_eq!(prefix, sets.as_slice());
    let complete = [sets.as_slice(), idr_slice];
    assert!(prefix_for(&complete, true, cached).unwrap().is_empty());
    assert!(prefix_for(&[p_slice], false, None).unwrap().is_empty());
}

#[test]
fn bildkette_task_linux_hw_entrypoint_and_profiles() {
    let full = |rc: u32, packed: u32| [va::VA_RT_FORMAT_YUV420, rc, packed, 8192, 8192];
    let (high, va_high) = (H264Profile::High, va::VA_PROFILE_H264_HIGH);
    let candidates = [
        (va::VA_ENTRYPOINT_ENC_SLICE, full(va::VA_RC_CBR, 0)),
        (va::VA_ENTRYPOINT_ENC_SLICE_LP, full(va::VA_RC_CBR, 3)),
    ];
    let caps = choose_encode(high, va_high, &candidates).unwrap();
    assert_eq!(caps.entrypoint, va::VA_ENTRYPOINT_ENC_SLICE_LP, "low power");
    assert_eq!(caps.packed_headers, 3);
    assert_eq!(caps.max, FrameSize::MAX);
    let no_cbr = [
        (va::VA_ENTRYPOINT_ENC_SLICE_LP, full(0x10, 3)),
        (va::VA_ENTRYPOINT_ENC_SLICE, full(va::VA_RC_CBR, 1)),
    ];
    let caps = choose_encode(high, va_high, &no_cbr).unwrap();
    let chosen = (caps.entrypoint, caps.packed_headers);
    assert_eq!(chosen, (va::VA_ENTRYPOINT_ENC_SLICE, 0));
    let unusable = [(va::VA_ENTRYPOINT_ENC_SLICE, full(0x10, 0))];
    assert!(choose_encode(high, va_high, &unusable).is_none());
    let limited = max_picture(VA_ATTRIB_NOT_SUPPORTED, 1080);
    assert_eq!(limited, FrameSize::new(4096, 1080));
    let (profile, decodes) = decode_profiles(&[va::VA_PROFILE_H264_MAIN]).unwrap();
    assert_eq!(profile, va::VA_PROFILE_H264_MAIN);
    assert_eq!(decodes, [H264Profile::ConstrainedBaseline]);
    let high_decodes = decode_profiles(&[va::VA_PROFILE_H264_HIGH]).unwrap().1;
    assert_eq!(high_decodes.len(), 2);
    assert!(decode_profiles(&[]).is_none());
}

#[test]
fn bildkette_task_linux_hw_dmabuf_interface() {
    let frame = DmaBufFrame {
        fd: 42,
        drm_format: u32::from_le_bytes(*b"XR24"),
        modifier: 0x0100_0000_0000_0001,
        size: FrameSize::new(1920, 1080),
        planes: vec![(0, 7680)],
        object_size: 7680 * 1080,
    };
    let descriptor = prime_descriptor(&frame, va::VA_FOURCC_BGRX).unwrap();
    assert_eq!((descriptor.num_objects, descriptor.num_layers), (1, 1));
    let object = descriptor.objects[0];
    assert_eq!((object.fd, descriptor.layers[0].pitch[0]), (42, 7680));
    assert_eq!(object.drm_format_modifier, frame.modifier);
    let empty = DmaBufFrame {
        planes: Vec::new(),
        ..frame
    };
    assert!(prime_descriptor(&empty, 0).is_err());
}

#[test]
fn bildkette_task_linux_hw_va_struct_sizes() {
    type Pipeline = va::VaProcPipelineParameterBuffer;
    assert_eq!(size_of::<va::VaConfigAttrib>(), 8);
    assert_eq!(size_of::<va::VaSurfaceAttrib>(), 24);
    assert_eq!(size_of::<va::VaImageFormat>(), 48);
    assert_eq!(size_of::<va::VaImage>(), 120);
    assert_eq!(size_of::<va::VaCodedBufferSegment>(), 48);
    assert_eq!(size_of::<Pipeline>(), 224);
    assert_eq!(offset_of!(Pipeline, input_color_properties), 132);
    assert_eq!(offset_of!(Pipeline, output_hdr_metadata), 152);
    assert_eq!(size_of::<va::VaDrmPrimeSurfaceDescriptor>(), 312);
    assert_eq!(size_of::<abi::VaPictureH264>(), 36);
    assert_eq!(size_of::<abi::VaEncSequenceH264>(), 1132);
    assert_eq!(offset_of!(abi::VaEncSequenceH264, offset_for_ref_frame), 44);
    assert_eq!(offset_of!(abi::VaEncSequenceH264, vui_fields), 1092);
    assert_eq!(size_of::<abi::VaEncPictureH264>(), 648);
    assert_eq!(offset_of!(abi::VaEncPictureH264, frame_num), 620);
    assert_eq!(offset_of!(abi::VaEncPictureH264, pic_fields), 628);
    assert_eq!(size_of::<abi::VaEncSliceH264>(), 3140);
    assert_eq!(offset_of!(abi::VaEncSliceH264, cabac_init_idc), 3118);
    assert_eq!(size_of::<abi::VaEncMiscRateControl>(), 60);
    assert_eq!(size_of::<abi::VaEncMiscHrd>(), 24);
    assert_eq!(size_of::<abi::VaEncMiscFrameRate>(), 24);
    assert_eq!(offset_of!(abi::VaEncMisc<abi::VaEncMiscHrd>, data), 4);
    assert_eq!(size_of::<abi::VaEncPackedHeaderParameter>(), 28);
    assert_eq!(size_of::<abi::VaPictureParameterH264>(), 672);
    assert_eq!(offset_of!(abi::VaPictureParameterH264, pic_fields), 632);
    assert_eq!(size_of::<abi::VaSliceParameterH264>(), 3128);
    assert_eq!(offset_of!(abi::VaSliceParameterH264, ref_pic_list0), 28);
    assert_eq!(
        offset_of!(abi::VaSliceParameterH264, chroma_weight_l0),
        2466
    );
    assert_eq!(size_of::<abi::VaIqMatrixH264>(), 240);
}

#[test]
fn bildkette_task_linux_hw_nvenc_struct_sizes() {
    assert_eq!(size_of::<nv::Guid>(), 16);
    assert_eq!(size_of::<nv::NvEncRcParams>(), 128);
    assert_eq!(size_of::<nv::NvEncVui>(), 112);
    assert_eq!(size_of::<nv::NvEncConfigH264>(), 1792);
    assert_eq!(size_of::<nv::NvEncConfig>(), 3584);
    assert_eq!(offset_of!(nv::NvEncConfig, rc_params), 40);
    assert_eq!(offset_of!(nv::NvEncConfig, h264), 168);
    assert_eq!(size_of::<nv::NvEncPresetConfig>(), 5128);
    assert_eq!(size_of::<nv::NvEncInitializeParams>(), 1808);
    assert_eq!(offset_of!(nv::NvEncInitializeParams, encode_config), 88);
    assert_eq!(offset_of!(nv::NvEncInitializeParams, tuning_info), 136);
    assert_eq!(size_of::<nv::NvEncReconfigureParams>(), 1824);
    assert_eq!(size_of::<nv::NvEncPicParams>(), 3360);
    assert_eq!(offset_of!(nv::NvEncPicParams, codec_pic_params), 80);
    assert_eq!(size_of::<nv::NvEncLockBitstream>(), 1552);
    assert_eq!(offset_of!(nv::NvEncLockBitstream, bitstream_buffer_ptr), 56);
    assert_eq!(size_of::<nv::NvEncCreateInputBuffer>(), 776);
    assert_eq!(size_of::<nv::NvEncCreateBitstreamBuffer>(), 776);
    assert_eq!(size_of::<nv::NvEncLockInputBuffer>(), 1544);
    assert_eq!(size_of::<nv::NvEncOpenSessionParams>(), 1552);
    assert_eq!(size_of::<nv::NvEncCapsParam>(), 256);
    assert_eq!(size_of::<nv::NvEncFunctions>(), 2552);
    assert_eq!(offset_of!(nv::NvEncFunctions, get_encode_caps), 64);
    assert_eq!(offset_of!(nv::NvEncFunctions, encode_picture), 136);
    assert_eq!(offset_of!(nv::NvEncFunctions, open_encode_session_ex), 240);
    assert_eq!(
        offset_of!(nv::NvEncFunctions, get_encode_preset_config_ex),
        320
    );
    assert_eq!(nv::NV_ENC_CONFIG_VER, 0xf108_000c);
    assert_eq!(nv::NV_ENC_PIC_PARAMS_VER, 0xf106_000c);
    assert_eq!(nv::NV_ENCODE_API_FUNCTION_LIST_VER, 0x7102_000c);
    assert!(nv::supported(0xc1) && nv::supported(0xd0) && !nv::supported(0xc0));
}

#[test]
fn bildkette_task_linux_hw_nvenc_settings() {
    let mut config = nv::NvEncConfig::zeroed();
    config.rc_params.flags = 1 << 5; // a preset with look-ahead
    let rate = RateParams::for_target(30_000_000, 120);
    apply_settings(&mut config, &rate);
    assert_eq!(config.gop_length, nv::NVENC_INFINITE_GOPLENGTH);
    assert_eq!(config.frame_interval_p, 1, "no B pictures");
    assert_eq!(config.profile_guid, nv::NV_ENC_H264_PROFILE_HIGH_GUID);
    let rc = &config.rc_params;
    assert_eq!(rc.rate_control_mode, nv::NV_ENC_PARAMS_RC_CBR);
    let bits = (rc.average_bit_rate, rc.max_bit_rate);
    assert_eq!(bits, (30_000_000, 30_000_000));
    let vbv = (rc.vbv_buffer_size, rc.vbv_initial_delay);
    assert_eq!(vbv, (250_000, 250_000));
    assert_eq!(rc.flags, nv::RC_ZERO_REORDER_DELAY);
    let repeat = config.h264.flags & nv::H264_REPEAT_SPSPPS;
    assert_eq!(repeat, nv::H264_REPEAT_SPSPPS);
    assert_eq!(config.h264.idr_period, nv::NVENC_INFINITE_GOPLENGTH);
    assert_eq!(config.h264.vui.colour_matrix, nv::VUI_BT709);
}
