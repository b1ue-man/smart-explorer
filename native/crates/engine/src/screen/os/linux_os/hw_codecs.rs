//! Hardware video codecs of Linux hosts and viewers (BK4: B-18, B-24,
//! B-38). The order is the one of plan 4.10a: the GPU vendor interfaces
//! first (NVENC, then VA-API per render node, as Sunshine probes them on
//! Linux, R01), then OpenH264 after the user's consent, then the tiles
//! (`codec_inventory.rs` appends those). Everything is found at run time:
//! a missing library, driver, render node or right leaves the list empty
//! and the register exactly as before; the only trace is the codec list.
//! Which coder a session uses is decided when it opens (`CodecRegistry`
//! tries the factories in this order with the session's size); a coder
//! that fails while encoding gives way to the next through the host
//! process's `refused` list, a VA-API decoder through `vaapi_decode`.
//! BK4b adds VA-API HEVC Main and AV1 Main (encoders and decoders) per
//! render node after NVENC (`video_ordered`); a failed HEVC/AV1 decoder is
//! locked through `vaapi_decode_core`.
use std::sync::Arc;

use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory};
use crate::screen::core::negotiate::CodecKind;

#[path = "av1_soft_codec.rs"]
mod av1_soft_codec;
#[path = "h264_bits.rs"]
mod h264_bits;
#[path = "hw_dmabuf_picture.rs"]
mod hw_dmabuf_picture;
#[path = "hw_encode_common.rs"]
mod hw_encode_common;
#[path = "nvenc_codecs.rs"]
mod nvenc_codecs;
#[path = "nvenc_device.rs"]
mod nvenc_device;
#[path = "nvenc_encode.rs"]
mod nvenc_encode;
#[path = "nvenc_ffi.rs"]
mod nvenc_ffi;
#[path = "vaapi_av1_abi.rs"]
mod vaapi_av1_abi;
#[path = "vaapi_av1_bits.rs"]
mod vaapi_av1_bits;
#[path = "vaapi_av1_decode.rs"]
mod vaapi_av1_decode;
#[path = "vaapi_av1_encode.rs"]
mod vaapi_av1_encode;
#[path = "vaapi_av1_filters.rs"]
mod vaapi_av1_filters;
#[path = "vaapi_av1_frame.rs"]
mod vaapi_av1_frame;
#[path = "vaapi_av1_headers.rs"]
mod vaapi_av1_headers;
#[path = "vaapi_av1_syntax.rs"]
mod vaapi_av1_syntax;
#[path = "vaapi_decode.rs"]
mod vaapi_decode;
#[path = "vaapi_decode_core.rs"]
mod vaapi_decode_core;
#[path = "vaapi_decode_params.rs"]
mod vaapi_decode_params;
#[path = "vaapi_display.rs"]
mod vaapi_display;
#[path = "vaapi_dmabuf.rs"]
mod vaapi_dmabuf;
#[path = "vaapi_encode.rs"]
mod vaapi_encode;
#[path = "vaapi_encode_base.rs"]
mod vaapi_encode_base;
#[path = "vaapi_ffi.rs"]
mod vaapi_ffi;
#[path = "vaapi_h264_abi.rs"]
mod vaapi_h264_abi;
#[path = "vaapi_h264_dpb.rs"]
mod vaapi_h264_dpb;
#[path = "vaapi_h264_headers.rs"]
mod vaapi_h264_headers;
#[path = "vaapi_h264_slice.rs"]
mod vaapi_h264_slice;
#[path = "vaapi_h264_syntax.rs"]
mod vaapi_h264_syntax;
#[path = "vaapi_hevc_abi.rs"]
mod vaapi_hevc_abi;
#[path = "vaapi_hevc_decode.rs"]
mod vaapi_hevc_decode;
#[path = "vaapi_hevc_decode_params.rs"]
mod vaapi_hevc_decode_params;
#[path = "vaapi_hevc_dpb.rs"]
mod vaapi_hevc_dpb;
#[path = "vaapi_hevc_encode.rs"]
mod vaapi_hevc_encode;
#[path = "vaapi_hevc_headers.rs"]
mod vaapi_hevc_headers;
#[path = "vaapi_hevc_params.rs"]
mod vaapi_hevc_params;
#[path = "vaapi_hevc_slice.rs"]
mod vaapi_hevc_slice;
#[path = "vaapi_hevc_syntax.rs"]
mod vaapi_hevc_syntax;
#[path = "vaapi_probe.rs"]
mod vaapi_probe;
#[path = "vaapi_surface.rs"]
mod vaapi_surface;
#[path = "vaapi_video_encode.rs"]
mod vaapi_video_encode;
#[path = "vaapi_video_probe.rs"]
mod vaapi_video_probe;
#[path = "vaapi_vpp.rs"]
mod vaapi_vpp;

#[cfg(test)]
#[path = "bildkette_hw_av1_decode_task_tests.rs"]
mod bildkette_hw_av1_decode_task_tests;
#[cfg(test)]
#[path = "bildkette_hw_av1_task_tests.rs"]
mod bildkette_hw_av1_task_tests;
#[cfg(test)]
#[path = "bildkette_hw_decode_task_tests.rs"]
mod bildkette_hw_decode_task_tests;
#[cfg(test)]
#[path = "bildkette_hw_hevc_decode_task_tests.rs"]
mod bildkette_hw_hevc_decode_task_tests;
#[cfg(test)]
#[path = "bildkette_hw_hevc_task_tests.rs"]
mod bildkette_hw_hevc_task_tests;
#[cfg(test)]
#[path = "bildkette_hw_task_tests.rs"]
mod bildkette_hw_task_tests;
#[cfg(test)]
#[path = "bildkette_w2_task_tests.rs"]
mod bildkette_w2_task_tests;

type Factories = (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>);

// The DMA-BUF capture of the PipeWire stream (BK8b, B-15) reaches the VA
// encoder through these: the picture, the hull that takes it, the import;
// merge W2: the GPU picture of the host pipeline and its exact read-back.
pub(super) use hw_dmabuf_picture::{read_bgrx, DmaBufCause, DmaBufPicture, DmaBufStream};
pub(super) use hw_encode_common::HeldDmaBuf;
pub(super) use vaapi_dmabuf::ImportTarget;
pub(super) use vaapi_vpp::DmaBufFrame;

/// What a DMA-BUF capture may offer (plan 2026-11-14, BK8b): the layouts
/// and modifiers the first VA-API encoder with a video processor imports.
/// Empty when NVENC exists (it ranks first, `ordered`, and takes CPU
/// pictures, so a DMA-BUF would be read back) or no VA device can convert
/// on the GPU. Probed once per process, only when a stream asks.
pub(super) fn dmabuf_import_target() -> ImportTarget {
    static TARGET: std::sync::OnceLock<ImportTarget> = std::sync::OnceLock::new();
    TARGET
        .get_or_init(|| {
            if nvenc_device::probe().is_some() {
                return ImportTarget::default();
            }
            // BK4b: a device with only HEVC/AV1 encoding imports as well.
            let device = vaapi_probe::devices().into_iter().find(|device| {
                device.video_proc && (!device.encode.is_empty() || !device.video_encode.is_empty())
            });
            match device {
                Some(device) => ImportTarget {
                    formats: vaapi_dmabuf::probe_node(&device.node),
                    node: Some(device.node.clone()),
                },
                None => ImportTarget::default(),
            }
        })
        .clone()
}

/// The hardware coders of this process in their order of choice; BK6 adds
/// NVENC HEVC and AV1 where the GPU reports them, BK4b VA-API HEVC and AV1
/// per render node after them (B-30, 4.10a: the vendor interface of the
/// GPU first, NVENC before VA-API as Sunshine orders them on Linux).
pub(super) fn codecs() -> Factories {
    let devices = vaapi_probe::devices();
    let (mut encoders, mut decoders) = ordered(nvenc_device::probe(), devices.clone());
    for codec in [CodecKind::Hevc, CodecKind::Av1] {
        if let Some(max) = nvenc_device::probe_codec(codec) {
            encoders.push(Arc::new(nvenc_codecs::NvencVideoFactory { codec, max }));
        }
    }
    let (video_encoders, video_decoders) = video_ordered(&devices);
    encoders.extend(video_encoders);
    decoders.extend(video_decoders);
    (encoders, decoders)
}

/// The VA-API HEVC and AV1 coders of `devices` (BK4b): per render node,
/// in the probe's order (HEVC, AV1); none without driver or profile.
fn video_ordered(devices: &[Arc<vaapi_probe::VaDevice>]) -> Factories {
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    let mut decoders: Vec<Arc<dyn DecoderFactory>> = Vec::new();
    for device in devices {
        for caps in &device.video_encode {
            encoders.push(Arc::new(vaapi_video_encode::VaVideoEncoderFactory {
                device: device.clone(),
                caps: caps.clone(),
            }));
        }
        for caps in &device.video_decode {
            decoders.push(Arc::new(vaapi_decode_core::VaVideoDecoderFactory {
                device: device.clone(),
                caps: caps.clone(),
            }));
        }
    }
    (encoders, decoders)
}

/// The software AV1 coders (BK6, B-25): SVT-AV1 and dav1d where the
/// system has them. Whether a device carries a size and rate is decided by
/// the encoding time this session measures (B-02, `rate_cadence.rs`).
pub(super) fn software_codecs() -> Factories {
    av1_soft_codec::codecs()
}

/// The order of plan 4.10a for an NVENC limit (if any) and VA devices.
fn ordered(
    nvenc: Option<crate::screen::core::pixel_convert::FrameSize>,
    devices: Vec<Arc<vaapi_probe::VaDevice>>,
) -> Factories {
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    let mut decoders: Vec<Arc<dyn DecoderFactory>> = Vec::new();
    if let Some(max) = nvenc {
        encoders.push(Arc::new(nvenc_encode::NvencEncoderFactory { max }));
    }
    for device in devices {
        if !device.encode.is_empty() {
            encoders.push(Arc::new(vaapi_encode::VaEncoderFactory {
                device: device.clone(),
            }));
        }
        if device.decode.is_some() {
            decoders.push(Arc::new(vaapi_decode::VaDecoderFactory { device }));
        }
    }
    (encoders, decoders)
}
