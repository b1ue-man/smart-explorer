//! Task tests of merge W1 (plan 2026-11-14 „Bildkette“, requests 2, 3 and
//! 8): NVENC and AMF in the codec list with H.264, HEVC and AV1, their
//! HEVC/AV1 settings, the parameter sets before a HEVC/AV1 key frame, and
//! the driver library from the driver store. No GPU is needed.
use std::mem::{offset_of, size_of};

use super::amf_codecs::{self as amf_names, AV1, AVC, HEVC};
use super::amf_settings::{dynamic_settings, key_settings, static_settings, QUERY_TIMEOUT_MS};
use super::nvenc_codecs::{self as nv_codecs, av1_view, hevc_view};
use super::nvenc_encode::{apply_codec_references, apply_codec_settings};
use super::nvenc_ffi::{self as nv, Zeroed};
use super::vendor_encode::{append_unit, temporal_delimiter, RateParams};
use super::*;
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_image::AdmittedBytes;
use crate::screen::os::dxgi_adapters::{
    find_in, in_driver_store, order_by_output, output_path, packed, store_folder,
    QueryRegistryInfo, OUTPUT_OFFSET,
};

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
fn bildkette_task_win_w1_vendor_codecs_in_the_codec_list() {
    let max = FrameSize::new(4096, 4096);
    let found = Found {
        nvenc: [Some(max), Some(max), Some(max)],
        amf: [Some(max), Some(max), None],
    };
    let rank = ranked(found, Vec::new());
    let names: Vec<String> = rank.iter().map(|factory| factory.name()).collect();
    assert_eq!(
        names,
        ["nvenc", "nvenc-hevc", "nvenc-av1", "amf", "amf-hevc"],
        "NVENC before AMF, H.264 before HEVC before AV1 (4.10a)"
    );
    let offers: Vec<CodecOffer> = rank.iter().flat_map(|factory| factory.offers()).collect();
    assert_eq!(
        offers[0],
        CodecOffer::h264(H264Profile::High, true).with_max(max)
    );
    assert_eq!(
        offers[1],
        CodecOffer::video(CodecKind::Hevc, false, true).with_max(max),
        "HEVC Main 4:2:0 in hardware"
    );
    assert_eq!(offers[2].codec, CodecKind::Av1);
    assert!(offers.iter().all(|offer| offer.hardware));
    // HEVC and AV1: 4:2:0 only, at least 64 pixels a side; H.264: High.
    let size = FrameSize::new(1920, 1080);
    for codec in [CodecKind::Hevc, CodecKind::Av1] {
        assert!(check_request(codec, max, &choice(codec, None), size).is_ok());
        let sharp = choice(codec, Some(H264Profile::High));
        assert!(check_request(codec, max, &sharp, size).is_err(), "4:4:4");
        let small = FrameSize::new(32, 32);
        assert!(check_request(codec, max, &choice(codec, None), small).is_err());
        let h264 = choice(CodecKind::H264, Some(H264Profile::High));
        assert!(
            check_request(codec, max, &h264, size).is_err(),
            "other codec"
        );
    }
    let high = choice(CodecKind::H264, Some(H264Profile::High));
    assert!(check_request(CodecKind::H264, max, &high, size).is_ok());
    assert!(check_request(CodecKind::Tiles, max, &high, size).is_err());
}

#[test]
fn bildkette_task_win_w1_vendor_nvenc_hevc_av1_settings() {
    let rate = RateParams::for_target(30_000_000, 120);
    for (codec, profile) in [
        (CodecKind::Hevc, nv_codecs::NV_ENC_HEVC_PROFILE_MAIN_GUID),
        (CodecKind::Av1, nv_codecs::NV_ENC_AV1_PROFILE_MAIN_GUID),
    ] {
        let mut config = nv::NvEncConfig::zeroed();
        config.rc_params.flags = nv::RC_ENABLE_LOOKAHEAD;
        apply_codec_settings(&mut config, &rate, codec);
        assert_eq!(config.profile_guid, profile, "{codec:?}");
        assert_eq!(config.gop_length, nv::NVENC_INFINITE_GOPLENGTH);
        assert_eq!(config.frame_interval_p, 1, "no B pictures");
        assert_eq!(config.rc_params.rate_control_mode, nv::NV_ENC_PARAMS_RC_CBR);
        assert_eq!(config.rc_params.vbv_buffer_size, 250_000, "one frame");
        assert_eq!(config.rc_params.flags, nv::RC_ZERO_REORDER_DELAY);
        apply_codec_references(&mut config, codec, true);
        assert_eq!(config.rc_params.low_delay_key_frame_scale, 1);
        assert!(nv_codecs::encode_guid(codec).is_some());
        match codec {
            CodecKind::Hevc => {
                let hevc = hevc_view(&mut config);
                assert_eq!(hevc.idr_period, nv::NVENC_INFINITE_GOPLENGTH);
                assert_eq!(hevc.flags & (1 << 7), 1 << 7, "repeatSPSPPS");
                assert_eq!((hevc.flags >> 9) & 0b11, 1, "4:2:0");
                assert_eq!(hevc.max_num_ref_frames_in_dpb, 5, "Sunshine DPB");
                assert_eq!(hevc.num_ref_l0, nv::NV_ENC_NUM_REF_FRAMES_1);
                assert_eq!(hevc.vui.colour_matrix, nv::VUI_BT709);
            }
            _ => {
                let av1 = av1_view(&mut config);
                assert_eq!(av1.idr_period, nv::NVENC_INFINITE_GOPLENGTH);
                assert_eq!(av1.flags & (1 << 5), 1 << 5, "repeatSeqHdr");
                assert_eq!(av1.flags & 1, 0, "low-overhead OBUs");
                assert_eq!(av1.max_num_ref_frames_in_dpb, 8, "Sunshine DPB");
                assert_eq!(av1.num_fwd_refs, nv::NV_ENC_NUM_REF_FRAMES_1);
                assert_eq!(av1.color_range, 0, "limited range");
            }
        }
    }
    assert_eq!(nv_codecs::factory_name(CodecKind::Av1), "nvenc-av1");
    assert_eq!(nv_codecs::encode_guid(CodecKind::Tiles), None);
}

#[test]
fn bildkette_task_win_w1_vendor_amf_hevc_av1_settings() {
    let size = FrameSize::new(2560, 1440);
    let rate = RateParams::for_target(40_000_000, 100);
    let int = |list: &[(&str, amf_abi::AmfVariant)], name: &str| {
        list.iter()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value.as_int64())
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    for names in [&HEVC, &AV1] {
        let settings = static_settings(names, size, &rate);
        assert_eq!(settings[0].0, names.usage, "Usage first");
        assert_eq!(int(&settings, names.usage), names.usage_ultra_low_latency);
        assert_eq!(int(&settings, names.rate_control), 3, "CBR");
        assert_eq!(int(&settings, names.quality), names.quality_speed);
        assert_eq!(int(&settings, names.target), 40_000_000);
        assert_eq!(int(&settings, names.peak), 40_000_000);
        assert_eq!(int(&settings, names.vbv), 400_000, "one frame");
        assert_eq!(int(&settings, names.max_ltr), 2);
        let after = dynamic_settings(names);
        assert!(after.iter().any(|(key, _)| *key == names.enforce_hrd));
        assert!(after.iter().any(|(key, _)| *key == names.skip));
        let key = key_settings(names);
        assert_eq!(key[0].0, names.force_type);
        assert_eq!(key[0].1.as_int64(), Some(names.force_key));
        assert_eq!(key.len(), 1 + names.insert_headers.len());
    }
    let hevc = static_settings(&HEVC, size, &rate);
    assert_eq!(HEVC.usage_ultra_low_latency, 1);
    assert_eq!(
        int(&hevc, amf_names::HEVC_HEADER_INSERTION),
        2,
        "IDR aligned"
    );
    assert_eq!(int(&hevc, amf_names::HEVC_GOP_SIZE), i64::from(i16::MAX));
    let av1 = static_settings(&AV1, size, &rate);
    assert_eq!(AV1.usage_ultra_low_latency, 2);
    assert_eq!(int(&av1, amf_names::AV1_LATENCY), 3, "lowest latency");
    assert_eq!(int(&av1, amf_names::AV1_ALIGNMENT), 3, "no padding");
    assert_eq!(int(&av1, AV1.query_timeout), QUERY_TIMEOUT_MS, "static");
    let av1_after = dynamic_settings(&AV1);
    assert_eq!(
        int(&av1_after, amf_names::AV1_GOP_SIZE),
        0,
        "key frames on request"
    );
    assert!(av1_after.iter().all(|(key, _)| *key != AV1.query_timeout));
    assert_eq!(AV1.min_version, amf_abi::full_version(1, 4, 30, 0));
    assert_eq!(amf_names::names(CodecKind::H264), Some(&AVC));
    assert_eq!(amf_names::names(CodecKind::Tiles), None);
    assert_eq!(Vendor::Amf.name(CodecKind::Hevc), "amf-hevc");
    assert_eq!(Vendor::Amf.name(CodecKind::H264), "amf");
}

/// One Annex B NAL unit of HEVC type `kind` with a one-byte payload.
fn nal(kind: u8, payload: u8) -> Vec<u8> {
    vec![0, 0, 0, 1, kind << 1, 1, payload]
}

/// One AV1 OBU of `kind` with its size field.
fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 0x02, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A grant with room for three access units of 4 KiB (`CodecOutput`).
fn output_grant() -> CaptureGrant {
    use crate::screen::core::capture_admission::{CaptureGeneration, CapturePlan, SourceKey};
    use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
    use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
    let name = "bildkette-w1-units";
    let pool = std::sync::Arc::new(MemoryPool::default());
    let ceiling = CapacityCeiling::bounded(MAX_WORKING_BYTES);
    let admission = pool
        .admit_session(None, ceiling, MAX_WORKING_BYTES, name)
        .unwrap();
    admission.bind_contract(None).unwrap();
    let key = SourceKey::new(name, "bildkette-fixture").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    let all = crate::screen::core::capture_plan::CapturePhases::ALL;
    plan.add_bytes(CaptureOwner::CodecOutput, 4096, 3, all)
        .unwrap();
    admission.authority().approve(plan).unwrap()
}

#[test]
fn bildkette_task_win_w1_vendor_key_frames_carry_their_parameter_sets() {
    let grant = output_grant();
    let unit = |parts: &[Vec<u8>]| parts.concat();
    let sets = unit(&[nal(32, 0x0c), nal(33, 0x0d), nal(34, 0x0e)]);
    let mut kept = None;
    let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, 4096).unwrap();
    // A first key frame without parameters is refused, not sent.
    let bare_idr = nal(19, 0xaf);
    assert!(append_unit(&mut bytes, 4096, CodecKind::Hevc, &mut kept, &bare_idr).is_err());
    let first = unit(&[sets.clone(), nal(19, 0xaf)]);
    assert!(append_unit(&mut bytes, 4096, CodecKind::Hevc, &mut kept, &first).unwrap());
    assert_eq!(kept.as_deref(), Some(sets.as_slice()));
    let trail = nal(1, 0xd0);
    assert!(!append_unit(&mut bytes, 4096, CodecKind::Hevc, &mut kept, &trail).unwrap());
    let start = bytes.len();
    assert!(append_unit(&mut bytes, 4096, CodecKind::Hevc, &mut kept, &bare_idr).unwrap());
    assert_eq!(
        &bytes.as_slice()[start..],
        unit(&[sets, bare_idr]).as_slice()
    );
    assert!(append_unit(&mut bytes, start, CodecKind::Hevc, &mut kept, &trail).is_err());
    // AV1: the sequence header goes after the temporal delimiter.
    let (delimiter, sequence) = (obu(2, &[]), obu(1, &[0x0a, 0x0b]));
    let (key, inter) = (obu(6, &[0x10]), obu(6, &[0x20]));
    assert_eq!(temporal_delimiter(&delimiter), 2);
    assert_eq!(temporal_delimiter(&key), 0);
    let mut kept = None;
    let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, 4096).unwrap();
    let first = unit(&[delimiter.clone(), sequence.clone(), key.clone()]);
    assert!(append_unit(&mut bytes, 4096, CodecKind::Av1, &mut kept, &first).unwrap());
    assert_eq!(kept.as_deref(), Some(sequence.as_slice()));
    let plain = unit(&[delimiter.clone(), inter]);
    assert!(!append_unit(&mut bytes, 4096, CodecKind::Av1, &mut kept, &plain).unwrap());
    let start = bytes.len();
    let again = unit(&[delimiter.clone(), key.clone()]);
    assert!(append_unit(&mut bytes, 4096, CodecKind::Av1, &mut kept, &again).unwrap());
    let expected = unit(&[delimiter, sequence, key]);
    assert_eq!(&bytes.as_slice()[start..], expected.as_slice());
    // H.264 passes unchanged.
    let mut kept = None;
    let mut bytes = AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, 64).unwrap();
    let idr = vec![0, 0, 0, 1, 0x65, 0x88];
    assert!(append_unit(&mut bytes, 64, CodecKind::H264, &mut kept, &idr).unwrap());
    assert_eq!(bytes.as_slice(), idr.as_slice());
    assert_eq!(kept, None);
}

#[test]
fn bildkette_task_win_w1_vendor_library_from_the_driver_store() {
    // The kernel's answer: a UTF-16 path behind the 544-byte header.
    assert_eq!(size_of::<QueryRegistryInfo>(), 552);
    assert_eq!(offset_of!(QueryRegistryInfo, output), OUTPUT_OFFSET);
    let path = r"C:\Windows\System32\DriverStore\FileRepository\nv_dispi.inf_amd64_0123";
    let text: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let mut words = vec![0u64; (OUTPUT_OFFSET + text.len() * 2).div_ceil(8)];
    let bytes: Vec<u8> = text.iter().flat_map(|unit| unit.to_ne_bytes()).collect();
    for (index, byte) in bytes.iter().enumerate() {
        let at = OUTPUT_OFFSET + index;
        let mut word = words[at / 8].to_ne_bytes();
        word[at % 8] = *byte;
        words[at / 8] = u64::from_ne_bytes(word);
    }
    let found = output_path(&words, bytes.len() as u32).expect("path");
    assert_eq!(found.to_string_lossy(), path);
    assert!(in_driver_store(&found));
    // Only the driver store, never another folder or a way out of it.
    assert!(!in_driver_store(std::path::Path::new(
        r"C:\Users\x\nvEncodeAPI64.dll"
    )));
    let out = format!(r"{path}\..\..\..\Temp");
    assert!(!in_driver_store(std::path::Path::new(&out)));
    // Only the folder name is taken, below this system's own directory: the
    // NT form (rooted, without a drive) lands there; another root, a way
    // out of the store and the store itself are refused.
    let system32 = std::path::Path::new(r"C:\Windows\System32");
    let repository = system32.join("DriverStore").join("FileRepository");
    let folder = repository.join("nv_dispi.inf_amd64_0123");
    assert_eq!(store_folder(&found, system32), Some(folder.clone()));
    let nt = r"\SystemRoot\System32\DriverStore\FileRepository\nv_dispi.inf_amd64_0123";
    assert_eq!(
        store_folder(std::path::Path::new(nt), system32),
        Some(folder)
    );
    let other = r"D:\x\System32\DriverStore\FileRepository\nv_dispi.inf_amd64_0123";
    assert_eq!(store_folder(std::path::Path::new(other), system32), None);
    assert_eq!(store_folder(std::path::Path::new(&out), system32), None);
    assert_eq!(store_folder(&repository, system32), None);
    // The library in the folder or one folder below it (AMD's build folder).
    let store = tempfile::tempdir().unwrap();
    assert_eq!(find_in(store.path(), "amfrt64.dll"), None);
    std::fs::create_dir(store.path().join("B123")).unwrap();
    std::fs::write(store.path().join("B123").join("amfrt64.dll"), b"x").unwrap();
    let below = store.path().join("B123").join("amfrt64.dll");
    assert_eq!(find_in(store.path(), "amfrt64.dll"), Some(below));
    std::fs::write(store.path().join("amfrt64.dll"), b"x").unwrap();
    let direct = store.path().join("amfrt64.dll");
    assert_eq!(find_in(store.path(), "amfrt64.dll"), Some(direct));
    // A missing library stays missing (no search path).
    let missing = dxgi_adapters::driver_library(VENDOR_NVIDIA, "se-w1-missing.dll");
    assert_eq!(
        missing.err().map(|error| error.kind()),
        Some(io::ErrorKind::NotFound)
    );
    // Adapters with a desktop output first, the order kept otherwise.
    let mut list = [(1, false), (2, true), (3, false), (4, true)];
    order_by_output(&mut list, |entry| entry.1);
    assert_eq!(list.map(|entry| entry.0), [2, 4, 1, 3]);
    let luid = windows::Win32::Foundation::LUID {
        LowPart: 0x1234,
        HighPart: 1,
    };
    assert_eq!(packed(luid), 0x0000_0001_0000_1234);
}
