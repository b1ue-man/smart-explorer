//! Plan 2026-11-17 P8: the FFmpeg loader's table and layout facts, the stage
//! order and the low-latency options, without FFmpeg or Windows.
use super::layout::*;
use super::*;

/// Offsets of a C struct of `(size, align)` fields on x86_64 (each field at
/// the next multiple of its alignment).
fn offsets(fields: &[(usize, usize)]) -> Vec<usize> {
    let mut at = 0;
    fields
        .iter()
        .map(|(size, align)| {
            at = at.div_ceil(*align) * align;
            let field = at;
            at += size;
            field
        })
        .collect()
}

const PTR: (usize, usize) = (8, 8);
const INT: (usize, usize) = (4, 4);
const I64: (usize, usize) = (8, 8);
const RATIONAL: (usize, usize) = (8, 4);

#[test]
fn bildkette_task_ffmpeg_symbol_table_and_versions() {
    for (index, symbol) in Symbol::ALL.iter().enumerate() {
        assert_eq!(*symbol as usize, index, "{symbol:?}");
        let name = symbol.name();
        let codec = name.starts_with("avcodec_")
            || name.starts_with("av_packet_")
            || name == "av_new_packet";
        let expected = if codec { AVCODEC } else { AVUTIL };
        assert_eq!(symbol.library(), expected, "{name}");
        assert!(!name.contains('\0'));
    }
    let mut names: Vec<&str> = Symbol::ALL.iter().map(|s| s.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Symbol::ALL.len(), "every symbol once");
    // Dependencies load first: avcodec imports avutil and swresample.
    assert_eq!(
        DLLS,
        ["avutil-60.dll", "swresample-6.dll", "avcodec-62.dll"]
    );
    assert_eq!(DLLS[AVUTIL], format!("avutil-{AVUTIL_MAJOR}.dll"));
    assert_eq!(DLLS[AVCODEC], format!("avcodec-{AVCODEC_MAJOR}.dll"));
    assert_eq!(DLLS[1], format!("swresample-{SWRESAMPLE_MAJOR}.dll"));
    // libavcodec 62.28.103 and libavutil 60.26.103 of the pinned build.
    let avcodec = (62 << 16) | (28 << 8) | 103;
    assert!(version_matches(avcodec, AVCODEC_MAJOR));
    assert!(!version_matches(
        (61 << 16) | (19 << 8) | 100,
        AVCODEC_MAJOR
    ));
    assert!(!version_matches((63 << 16) | 1, AVCODEC_MAJOR));
    assert_eq!(version_text(avcodec), "62.28.103");
}

#[test]
fn bildkette_task_ffmpeg_layouts_follow_the_pinned_headers() {
    // AVFrame, libavutil/frame.h:427-698 of n8.1.3 (no conditional fields).
    let frame = offsets(&[
        (64, 8), // data[8]
        (32, 4), // linesize[8]
        PTR,     // extended_data
        INT,     // width
        INT,     // height
        INT,     // nb_samples
        INT,     // format
        INT,     // pict_type
        RATIONAL,
        I64, // pts
        I64, // pkt_dts
        RATIONAL,
        INT,     // quality
        PTR,     // opaque
        INT,     // repeat_pict
        INT,     // sample_rate
        (64, 8), // buf[8]
        PTR,     // extended_buf
        INT,     // nb_extended_buf
        PTR,     // side_data
        INT,     // nb_side_data
        INT,     // flags
        INT,     // color_range
        INT,     // color_primaries
        INT,     // color_trc
        INT,     // colorspace
        INT,     // chroma_location
        I64,     // best_effort_timestamp
    ]);
    assert_eq!(
        [frame[0], frame[1], frame[2], frame[3], frame[4], frame[6]],
        [
            FRAME_DATA,
            FRAME_LINESIZE,
            FRAME_EXTENDED_DATA,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            FRAME_FORMAT
        ]
    );
    assert_eq!(
        [frame[8], frame[9], frame[10], frame[11]],
        [
            FRAME_SAMPLE_ASPECT,
            FRAME_PTS,
            FRAME_PKT_DTS,
            FRAME_TIME_BASE
        ]
    );
    assert_eq!(
        [frame[22], frame[23], frame[24], frame[25], frame[27]],
        [
            FRAME_COLOR_RANGE,
            FRAME_COLOR_PRIMARIES,
            FRAME_COLOR_TRC,
            FRAME_COLORSPACE,
            FRAME_BEST_EFFORT
        ]
    );
    assert_eq!(FRAME_PROBE, FRAME_BEST_EFFORT + 8);
    // AVPacket, libavcodec/packet.h:565-633.
    let packet = offsets(&[
        PTR, I64, I64, PTR, INT, INT, INT, PTR, INT, I64, I64, PTR, PTR, RATIONAL,
    ]);
    assert_eq!(
        [packet[0], packet[1], packet[2], packet[3], packet[4], packet[6]],
        [
            PACKET_BUF,
            PACKET_PTS,
            PACKET_DTS,
            PACKET_DATA,
            PACKET_SIZE,
            PACKET_FLAGS
        ]
    );
    assert_eq!(
        [packet[10], packet[13], packet[13] + 8],
        [PACKET_POS, PACKET_TIME_BASE, PACKET_PROBE]
    );
    // AVBufferRef, AVHWDeviceContext, AVD3D11VADeviceContext, AVOption.
    assert_eq!(offsets(&[PTR, PTR, PTR])[1], BUFFER_DATA);
    assert_eq!(offsets(&[PTR, INT, PTR, PTR, PTR])[2], HWDEVICE_HWCTX);
    let d3d11 = offsets(&[PTR, PTR, PTR, PTR, PTR, PTR, PTR, INT, INT]);
    assert_eq!(
        [d3d11[0], d3d11[1], d3d11[4], d3d11[5], d3d11[6]],
        [
            D3D11VA_DEVICE,
            D3D11VA_DEVICE_CONTEXT,
            D3D11VA_LOCK,
            D3D11VA_UNLOCK,
            D3D11VA_LOCK_CTX
        ]
    );
    assert_eq!(offsets(&[PTR, PTR, INT, INT])[2], OPTION_OFFSET);
}

/// The defaults `av_frame_alloc` and `av_packet_alloc` write, at the
/// offsets the loader checks.
#[test]
fn bildkette_task_ffmpeg_fresh_frame_and_packet_fingerprints() {
    let address = 0x1234_5678_9abc_def0_u64;
    let mut frame = vec![0u8; FRAME_PROBE];
    let mut put = |at: usize, value: &[u8]| frame[at..at + value.len()].copy_from_slice(value);
    put(FRAME_EXTENDED_DATA, &address.to_le_bytes());
    put(FRAME_FORMAT, &(-1i32).to_le_bytes());
    put(FRAME_SAMPLE_ASPECT + 4, &1i32.to_le_bytes());
    put(FRAME_PTS, &NOPTS.to_le_bytes());
    put(FRAME_PKT_DTS, &NOPTS.to_le_bytes());
    put(FRAME_TIME_BASE + 4, &1i32.to_le_bytes());
    for at in [FRAME_COLOR_PRIMARIES, FRAME_COLOR_TRC, FRAME_COLORSPACE] {
        put(at, &2i32.to_le_bytes());
    }
    put(FRAME_BEST_EFFORT, &NOPTS.to_le_bytes());
    assert!(fresh_frame_ok(&frame, address));
    assert!(
        !fresh_frame_ok(&frame, address + 8),
        "extended_data elsewhere"
    );
    assert!(!fresh_frame_ok(&frame[..FRAME_PROBE - 1], address));
    let mut shifted = frame.clone();
    shifted[FRAME_PTS..FRAME_PTS + 8].copy_from_slice(&0i64.to_le_bytes());
    assert!(!fresh_frame_ok(&shifted, address), "pts not where expected");

    let mut packet = vec![0u8; PACKET_PROBE];
    packet[PACKET_PTS..PACKET_PTS + 8].copy_from_slice(&NOPTS.to_le_bytes());
    packet[PACKET_DTS..PACKET_DTS + 8].copy_from_slice(&NOPTS.to_le_bytes());
    packet[PACKET_POS..PACKET_POS + 8].copy_from_slice(&(-1i64).to_le_bytes());
    packet[PACKET_TIME_BASE + 4..PACKET_TIME_BASE + 8].copy_from_slice(&1i32.to_le_bytes());
    assert!(fresh_packet_ok(&packet));
    packet[PACKET_SIZE] = 1;
    assert!(!fresh_packet_ok(&packet));

    // hw_device_ctx stands 8 bytes before hwaccel_flags, extra_hw_frames 4 after.
    assert_eq!(hw_device_offset(1080, 1084), Some(1072));
    assert_eq!(hw_device_offset(1080, 1088), None);
    assert_eq!(hw_device_offset(1084, 1088), None);
    assert_eq!(hw_device_offset(8, 12), None);
}

#[test]
fn bildkette_task_ffmpeg_stage_order_names_and_options() {
    use FfmpegStage::*;
    assert_eq!(
        ranking(DecoderPreference::Automatic),
        &[D3d11va, Dxva2, Software]
    );
    assert_eq!(ranking(DecoderPreference::Hardware), &[D3d11va, Dxva2]);
    assert_eq!(ranking(DecoderPreference::Software), &[Software]);
    let auto = DecoderPreference::Automatic;
    assert_eq!(
        stages(auto, CodecKind::H264, true, true),
        vec![D3d11va, Dxva2, Software]
    );
    assert_eq!(
        stages(auto, CodecKind::Hevc, false, true),
        vec![Dxva2, Software]
    );
    assert_eq!(stages(auto, CodecKind::Av1, true, false), vec![Software]);
    assert!(stages(DecoderPreference::Hardware, CodecKind::H264, true, false).is_empty());
    assert!(stages(auto, CodecKind::Tiles, false, false).is_empty());
    assert_eq!(decoder_name(CodecKind::Av1, Software), Some("libdav1d"));
    assert_eq!(decoder_name(CodecKind::Av1, D3d11va), Some("av1"));
    assert_eq!(decoder_name(CodecKind::Hevc, Dxva2), Some("hevc"));
    assert_eq!(
        stage_name(CodecKind::H264, D3d11va),
        "FFmpeg h264 [D3D11VA]"
    );
    assert_eq!(stage_name(CodecKind::Hevc, Dxva2), "FFmpeg hevc [DXVA2]");
    assert_eq!(
        stage_name(CodecKind::Av1, Software),
        "FFmpeg av1 [Software]"
    );
    assert_eq!(
        [
            D3d11va.device_type(),
            Dxva2.device_type(),
            Software.device_type()
        ],
        [Some("d3d11va"), Some("dxva2"), None]
    );
    // No option adds a picture of delay: low_delay always, no frame threads.
    for stage in [D3d11va, Dxva2, Software] {
        let options = decoder_options(stage, 16);
        assert!(options.contains(&("flags", OptionValue::Text("+low_delay"))));
        assert!(options.contains(&("err_detect", OptionValue::Text("+explode"))));
        assert!(!options
            .iter()
            .any(|(_, value)| *value == OptionValue::Text("frame")));
    }
    let hardware = decoder_options(D3d11va, 16);
    assert!(hardware.contains(&("threads", OptionValue::Int(1))));
    assert!(hardware.contains(&("extra_hw_frames", OptionValue::Int(EXTRA_HW_FRAMES))));
    let software = decoder_options(Software, 16);
    assert!(software.contains(&("thread_type", OptionValue::Text("slice"))));
    assert!(software.contains(&("threads", OptionValue::Int(4))));
    assert!(decoder_options(Software, 0).contains(&("threads", OptionValue::Int(1))));
    // Output formats and the stage that may hand them out.
    let formats = PixelFormats {
        d3d11: 172,
        dxva2: 51,
        nv12: 23,
        yuv420p: 0,
    };
    assert!(formats.valid());
    assert!(!PixelFormats { nv12: 0, ..formats }.valid());
    assert!(!PixelFormats {
        d3d11: -1,
        ..formats
    }
    .valid());
    assert_eq!(formats.kind(172), FrameKind::D3d11);
    assert_eq!(formats.kind(0), FrameKind::Yuv420p);
    assert_eq!(formats.kind(-1), FrameKind::Other);
    assert!(frame_fits_stage(FrameKind::D3d11, D3d11va));
    assert!(
        !frame_fits_stage(FrameKind::Yuv420p, D3d11va),
        "silent fallback"
    );
    assert!(frame_fits_stage(FrameKind::Dxva2, Dxva2));
    assert!(frame_fits_stage(FrameKind::Nv12, Software));
    assert!(!frame_fits_stage(FrameKind::D3d11, Software));
    assert_eq!(yuv_matrix(1), YuvMatrix::Bt709);
    assert_eq!(yuv_matrix(2), YuvMatrix::Bt709);
    assert_eq!(yuv_matrix(5), YuvMatrix::Bt601);
    assert_eq!(yuv_matrix(6), YuvMatrix::Bt601);
    assert!(full_range(2) && !full_range(1) && !full_range(0));
    assert_eq!(
        AVERROR_EOF,
        -(0x45 | (0x4F << 8) | (0x46 << 16) | (0x20 << 24))
    );
}
