//! FFmpeg as the decoder of the Windows viewer (plan 2026-11-17
//! „Windows-Bild“ P8, the way Moonlight and Shadow decode): the pinned build,
//! the functions the loader takes from it, the layout facts it checks before
//! the first call, the decoder stages and their order, and the options of a
//! decoder without added delay. Pure data and decisions without Windows or
//! FFmpeg, so every rule is tested on every platform; loading and decoding
//! are `os/windows/ffmpeg_*.rs`.
//!
//! Build: BtbN FFmpeg-Builds release `autobuild-2026-10-31-12-59`, archive
//! `ffmpeg-n8.1.3-27-g8dca1586f4-win64-lgpl-shared-8.1.zip`, configured with
//! `--enable-version3` and without `--enable-gpl` and `--enable-nonfree`
//! (LGPL v3; the configure line is in `avutil-60.dll`). libavcodec 62.28.103,
//! libavutil 60.26.103 (`include/libavcodec/version.h:32-33`,
//! `version_major.h:28`, `include/libavutil/version.h:81-83` of that
//! archive). `native/ffmpeg-runtime.nsh` pins URL, size and hashes.
//!
//! Layouts (x86_64 Windows, LLP64) from the headers of that archive:
//! `AVFrame` `libavutil/frame.h:427-783`, `AVPacket`
//! `libavcodec/packet.h:565-633`, `AVBufferRef` `libavutil/buffer.h:82-95`,
//! `AVHWDeviceContext` `libavutil/hwcontext.h:63-106`,
//! `AVD3D11VADeviceContext` `libavutil/hwcontext_d3d11va.h:45-111`, `AVOption`
//! `libavutil/opt.h:429-480`. The loader does not trust them blindly: a fresh
//! frame and a fresh packet must show the defaults of `get_frame_defaults`
//! (`libavutil/frame.c:31-50` of n8.1.3) and `get_packet_defaults`
//! (`libavcodec/packet.c:53-61`) at these offsets, and the offset of
//! `AVCodecContext.hw_device_ctx` comes from the options table at run time:
//! `hwaccel_flags` and `extra_hw_frames` follow it (`avcodec.h:1487-1510`).
use super::negotiate::{CodecKind, DecoderPreference};
use super::pixel_convert::YuvMatrix;

/// The FFmpeg release of the pinned build.
pub const FFMPEG_RELEASE: &str = "n8.1.3-27-g8dca1586f4";
pub const AVUTIL_MAJOR: u32 = 60;
pub const SWRESAMPLE_MAJOR: u32 = 6;
pub const AVCODEC_MAJOR: u32 = 62;
/// The DLLs in load order: each one's imports load before it
/// (`avcodec-62.dll` imports `avutil-60.dll` and `swresample-6.dll`,
/// `swresample-6.dll` imports `avutil-60.dll`; PE import tables).
pub const DLLS: [&str; 3] = ["avutil-60.dll", "swresample-6.dll", "avcodec-62.dll"];
/// Index of a DLL in `DLLS`.
pub const AVUTIL: usize = 0;
pub const AVCODEC: usize = 2;

/// Whether `version` (`major << 16 | minor << 8 | micro`, `avutil_version`,
/// `avcodec_version`) belongs to `major`.
pub fn version_matches(version: u32, major: u32) -> bool {
    version >> 16 == major
}

/// `62.28.103`.
pub fn version_text(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 16,
        (version >> 8) & 0xFF,
        version & 0xFF
    )
}

/// The functions the loader takes, each from its DLL; `Symbol as usize` is
/// its index in the loader's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Symbol {
    AvutilVersion,
    FrameAlloc,
    FrameFree,
    FrameRef,
    FrameUnref,
    BufferRef,
    BufferUnref,
    OptSet,
    OptSetInt,
    OptGetInt,
    OptFind,
    HwdeviceCtxAlloc,
    HwdeviceCtxInit,
    HwdeviceCtxCreate,
    HwdeviceFindTypeByName,
    HwframeTransferData,
    GetPixFmt,
    Strerror,
    LogSetLevel,
    AvcodecVersion,
    PacketAlloc,
    PacketFree,
    NewPacket,
    PacketUnref,
    FindDecoderByName,
    AllocContext3,
    FreeContext,
    Open2,
    SendPacket,
    ReceiveFrame,
}

impl Symbol {
    pub const ALL: [Self; 30] = [
        Self::AvutilVersion,
        Self::FrameAlloc,
        Self::FrameFree,
        Self::FrameRef,
        Self::FrameUnref,
        Self::BufferRef,
        Self::BufferUnref,
        Self::OptSet,
        Self::OptSetInt,
        Self::OptGetInt,
        Self::OptFind,
        Self::HwdeviceCtxAlloc,
        Self::HwdeviceCtxInit,
        Self::HwdeviceCtxCreate,
        Self::HwdeviceFindTypeByName,
        Self::HwframeTransferData,
        Self::GetPixFmt,
        Self::Strerror,
        Self::LogSetLevel,
        Self::AvcodecVersion,
        Self::PacketAlloc,
        Self::PacketFree,
        Self::NewPacket,
        Self::PacketUnref,
        Self::FindDecoderByName,
        Self::AllocContext3,
        Self::FreeContext,
        Self::Open2,
        Self::SendPacket,
        Self::ReceiveFrame,
    ];

    /// The exported name (`lib/avutil-60.def`, `lib/avcodec-62.def`).
    pub fn name(self) -> &'static str {
        match self {
            Self::AvutilVersion => "avutil_version",
            Self::FrameAlloc => "av_frame_alloc",
            Self::FrameFree => "av_frame_free",
            Self::FrameRef => "av_frame_ref",
            Self::FrameUnref => "av_frame_unref",
            Self::BufferRef => "av_buffer_ref",
            Self::BufferUnref => "av_buffer_unref",
            Self::OptSet => "av_opt_set",
            Self::OptSetInt => "av_opt_set_int",
            Self::OptGetInt => "av_opt_get_int",
            Self::OptFind => "av_opt_find",
            Self::HwdeviceCtxAlloc => "av_hwdevice_ctx_alloc",
            Self::HwdeviceCtxInit => "av_hwdevice_ctx_init",
            Self::HwdeviceCtxCreate => "av_hwdevice_ctx_create",
            Self::HwdeviceFindTypeByName => "av_hwdevice_find_type_by_name",
            Self::HwframeTransferData => "av_hwframe_transfer_data",
            Self::GetPixFmt => "av_get_pix_fmt",
            Self::Strerror => "av_strerror",
            Self::LogSetLevel => "av_log_set_level",
            Self::AvcodecVersion => "avcodec_version",
            Self::PacketAlloc => "av_packet_alloc",
            Self::PacketFree => "av_packet_free",
            Self::NewPacket => "av_new_packet",
            Self::PacketUnref => "av_packet_unref",
            Self::FindDecoderByName => "avcodec_find_decoder_by_name",
            Self::AllocContext3 => "avcodec_alloc_context3",
            Self::FreeContext => "avcodec_free_context",
            Self::Open2 => "avcodec_open2",
            Self::SendPacket => "avcodec_send_packet",
            Self::ReceiveFrame => "avcodec_receive_frame",
        }
    }

    /// The DLL (index in `DLLS`) that exports it.
    pub fn library(self) -> usize {
        if (self as usize) < Self::AvcodecVersion as usize {
            AVUTIL
        } else {
            AVCODEC
        }
    }
}

#[path = "ffmpeg_layout.rs"]
mod layout_checks;
pub use layout_checks::{
    fresh_frame_ok, fresh_packet_ok, hw_device_offset, layout, read_i32, read_i64, read_u64, NOPTS,
};

/// `AVERROR(EAGAIN)` with the Windows C runtime (`EAGAIN` 11).
pub const AVERROR_EAGAIN: i32 = -11;
/// `AVERROR_EOF` = `FFERRTAG('E','O','F',' ')` (`libavutil/error.h:57`).
pub const AVERROR_EOF: i32 = -0x2046_4F45;
/// `AV_LOG_ERROR` (`libavutil/log.h:210`).
pub const AV_LOG_ERROR: i32 = 16;
/// `AV_PKT_FLAG_KEY` (`libavcodec/packet.h:643`).
pub const AV_PKT_FLAG_KEY: i32 = 1;
/// Pool surfaces the decoder adds for the viewer (`extra_hw_frames`): the
/// picture on screen, the one waiting for it and one being presented, plus
/// one, so a held picture never starves the decoder's pool.
pub const EXTRA_HW_FRAMES: i64 = 4;
/// Slice threads of the software stage at most (Moonlight's `MAX_SLICES`
/// is 4; a host's encoder writes one slice per row group).
pub const MAX_SLICE_THREADS: usize = 4;

/// One stage of the FFmpeg decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FfmpegStage {
    /// D3D11VA on the viewer's Direct3D 11 device: the picture stays a
    /// texture of the device the presenter draws with.
    D3d11va,
    /// DXVA2 on a Direct3D 9 device of FFmpeg; the picture is read back.
    Dxva2,
    /// FFmpeg's software decoders (H.264, HEVC; AV1 with libdav1d).
    Software,
}

impl FfmpegStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::D3d11va => "d3d11va",
            Self::Dxva2 => "dxva2",
            Self::Software => "software",
        }
    }

    /// Text of the statistics and of `se screen codec`.
    pub fn label(self) -> &'static str {
        match self {
            Self::D3d11va => "D3D11VA",
            Self::Dxva2 => "DXVA2",
            Self::Software => "Software",
        }
    }

    pub fn hardware(self) -> bool {
        self != Self::Software
    }

    /// FFmpeg's name of the hardware device type (`av_hwdevice_find_type_by_name`).
    pub fn device_type(self) -> Option<&'static str> {
        match self {
            Self::D3d11va => Some("d3d11va"),
            Self::Dxva2 => Some("dxva2"),
            Self::Software => None,
        }
    }
}

/// The stages of the viewer's choice „Dekodierer“ (4.10) in their order:
/// D3D11VA, then DXVA2, then software.
pub fn ranking(preference: DecoderPreference) -> &'static [FfmpegStage] {
    match preference {
        DecoderPreference::Automatic => &[
            FfmpegStage::D3d11va,
            FfmpegStage::Dxva2,
            FfmpegStage::Software,
        ],
        DecoderPreference::Hardware => &[FfmpegStage::D3d11va, FfmpegStage::Dxva2],
        DecoderPreference::Software => &[FfmpegStage::Software],
    }
}

/// The stages a decoder opens with, in order: D3D11VA only where the
/// viewer's device decodes the stream (`d3d11_fits`), hardware only while
/// this process has not given it up (`hardware_allowed`), software where
/// FFmpeg has a decoder of the codec.
pub fn stages(
    preference: DecoderPreference,
    codec: CodecKind,
    d3d11_fits: bool,
    hardware_allowed: bool,
) -> Vec<FfmpegStage> {
    ranking(preference)
        .iter()
        .copied()
        .filter(|stage| match stage {
            FfmpegStage::D3d11va => d3d11_fits && hardware_allowed,
            FfmpegStage::Dxva2 => hardware_allowed,
            FfmpegStage::Software => decoder_name(codec, *stage).is_some(),
        })
        .collect()
}

/// FFmpeg's decoder of `codec` for `stage` (`avcodec_find_decoder_by_name`):
/// the native decoders take the hardware; AV1 in software is libdav1d, as
/// FFmpeg's own AV1 decoder decodes only with a hardware accelerator.
pub fn decoder_name(codec: CodecKind, stage: FfmpegStage) -> Option<&'static str> {
    match (codec, stage) {
        (CodecKind::H264, _) => Some("h264"),
        (CodecKind::Hevc, _) => Some("hevc"),
        (CodecKind::Av1, FfmpegStage::Software) => Some("libdav1d"),
        (CodecKind::Av1, _) => Some("av1"),
        (CodecKind::Tiles, _) => None,
    }
}

/// Name of an opened decoder for the statistics: `FFmpeg h264 [D3D11VA]`.
/// „D3D11VA“ in it makes the display path a decoder texture
/// (`G/screen_present.rs` `DecodedSurface::of_decoder`).
pub fn stage_name(codec: CodecKind, stage: FfmpegStage) -> String {
    format!("FFmpeg {} [{}]", codec.key(), stage.label())
}

/// Name of the factory of `codec` in `se screen codec`.
pub fn factory_name(codec: CodecKind) -> String {
    format!("FFmpeg {} (D3D11VA/DXVA2/Software)", codec.key())
}

/// One option of the decoder context, set with `av_opt_set`/`av_opt_set_int`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionValue {
    Text(&'static str),
    Int(i64),
}

/// The options of a decoder without added delay (Moonlight-qt
/// `app/streaming/video/ffmpeg.cpp:509-542` at 28629dc1): `low_delay`
/// (`AV_CODEC_FLAG_LOW_DELAY`), errors reported (`err_detect explode`: the
/// viewer then asks for a key frame instead of showing damage), hardware
/// with one thread and the extra pool surfaces the presenter holds,
/// software with slice threads only (frame threads hold one picture per
/// thread back). Names from `libavcodec/options_table.h:56,70,134,138,215,
/// 366-367,406` of n8.1.3.
pub fn decoder_options(stage: FfmpegStage, cpus: usize) -> Vec<(&'static str, OptionValue)> {
    let mut options = vec![
        ("flags", OptionValue::Text("+low_delay")),
        ("err_detect", OptionValue::Text("+explode")),
    ];
    if stage.hardware() {
        options.push(("threads", OptionValue::Int(1)));
        options.push(("extra_hw_frames", OptionValue::Int(EXTRA_HW_FRAMES)));
    } else {
        options.push(("thread_type", OptionValue::Text("slice")));
        let threads = cpus.clamp(1, MAX_SLICE_THREADS) as i64;
        options.push(("threads", OptionValue::Int(threads)));
    }
    options
}

/// The matrix of a stream's `colorspace` (`AVColorSpace`: 5 BT470BG and 6
/// SMPTE170M are BT.601, `libavutil/pixfmt.h:706-707`); BT.709 otherwise.
pub fn yuv_matrix(colorspace: i64) -> YuvMatrix {
    if matches!(colorspace, 5 | 6) {
        YuvMatrix::Bt601
    } else {
        YuvMatrix::Bt709
    }
}

/// Whether `color_range` is full range (`AVCOL_RANGE_JPEG` = 2,
/// `libavutil/pixfmt.h:777`); se's hosts encode limited range.
pub fn full_range(color_range: i64) -> bool {
    color_range == 2
}

/// What a decoded frame holds, from its `format` and the run-time codes of
/// the pixel formats (`av_get_pix_fmt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    D3d11,
    Dxva2,
    Nv12,
    Yuv420p,
    Other,
}

/// Run-time codes of the pixel formats the decoder tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormats {
    pub d3d11: i32,
    pub dxva2: i32,
    pub nv12: i32,
    pub yuv420p: i32,
}

impl PixelFormats {
    /// FFmpeg's names (`libavutil/pixdesc.c`): `d3d11`, `dxva2_vld`, `nv12`,
    /// `yuv420p`.
    pub const NAMES: [&'static str; 4] = ["d3d11", "dxva2_vld", "nv12", "yuv420p"];

    pub fn kind(&self, format: i32) -> FrameKind {
        match format {
            _ if format < 0 => FrameKind::Other,
            f if f == self.d3d11 => FrameKind::D3d11,
            f if f == self.dxva2 => FrameKind::Dxva2,
            f if f == self.nv12 => FrameKind::Nv12,
            f if f == self.yuv420p => FrameKind::Yuv420p,
            _ => FrameKind::Other,
        }
    }

    /// Every code was found and they differ.
    pub fn valid(&self) -> bool {
        let codes = [self.d3d11, self.dxva2, self.nv12, self.yuv420p];
        codes.iter().all(|code| *code >= 0)
            && (1..codes.len()).all(|i| !codes[..i].contains(&codes[i]))
    }
}

/// Whether a frame of `kind` fits the stage that decoded it: a hardware
/// stage that hands out a software picture fell back inside FFmpeg (the
/// default `get_format` takes software when the accelerator does not open,
/// `libavcodec/decode.c:993-1054`), which the viewer's choice „Hardware“
/// must not take and the statistics must not call hardware.
pub fn frame_fits_stage(kind: FrameKind, stage: FfmpegStage) -> bool {
    match stage {
        FfmpegStage::D3d11va => kind == FrameKind::D3d11,
        FfmpegStage::Dxva2 => kind == FrameKind::Dxva2,
        FfmpegStage::Software => matches!(kind, FrameKind::Nv12 | FrameKind::Yuv420p),
    }
}

#[cfg(test)]
#[path = "ffmpeg_plan_tests.rs"]
mod tests;
