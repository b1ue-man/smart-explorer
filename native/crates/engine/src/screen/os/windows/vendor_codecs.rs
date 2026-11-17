//! NVENC and AMF directly on Windows (plan 2026-11-14 bildkette, package
//! BK12, decision B-22) and their place in the rank of plan 4.10a: the
//! vendor interface of the capture's GPU first, then the Media Foundation
//! hardware MFTs, then the software MFT (and after them OpenH264 and the
//! tiles, which `codec_inventory.rs` appends).
//!
//! - Found at run time: `nvEncodeAPI64.dll` and `amfrt64.dll` load from
//!   System32, else from the driver store folder of the vendor's adapter
//!   (`dxgi_adapters::driver_library`, merge W1); a probe per process opens
//!   a session on the first hardware adapter of the vendor (an adapter with
//!   a desktop output first) and reads the largest picture per codec
//!   (H.264, and by merge W1 HEVC and AV1, B-23/B-30). Without the library,
//!   the adapter, a new enough driver or a session nothing is listed and
//!   nothing changes; the only trace is the codec list (`se screen codec
//!   --json`, with the settings an AMF component refused).
//! - Opened per session on the capture's own device
//!   (`d3d_input::attach_device` with the vendor's PCI id): textures of the
//!   capture never cross adapters (BK2a E4). Plan 2026-11-17 P3 (b): with
//!   the capture on another adapter or vendor (a VM's display adapter, the
//!   iGPU of a hybrid laptop) or no capture device yet, a device of its own
//!   on the vendor's adapter with the CPU upload of every picture
//!   (`d3d_input::own_for_vendor`); without admission (the run-time check,
//!   `bench --local`) always such a device. A GPU path switched off after a
//!   failure still refuses, and the next encoder of the rank opens.
//! - A vendor encoder that fails while encoding gives way to the next one
//!   through the host process's `refused` list (`host_worker_produce.rs`).
//! - Hooked into the codec list by `codec_facade.rs` (Windows
//!   `media_foundation_codecs`, merge W1).
use std::io;
use std::sync::{Arc, OnceLock};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};

use super::dxgi_adapters;
use super::mf_encoder::d3d_input::{self, GpuInput, InputRoute};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::codec_registry::EncoderFactory;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

#[path = "amf_abi.rs"]
mod amf_abi;
#[path = "amf_codecs.rs"]
mod amf_codecs;
#[path = "amf_encode.rs"]
mod amf_encode;
#[path = "amf_settings.rs"]
mod amf_settings;
#[cfg(test)]
#[path = "bildkette_vendor_task_tests.rs"]
mod bildkette_vendor_task_tests;
#[cfg(test)]
#[path = "bildkette_verify_win_task_tests.rs"]
mod bildkette_verify_win_task_tests;
#[cfg(test)]
#[path = "bildkette_w1_vendor_task_tests.rs"]
mod bildkette_w1_vendor_task_tests;
#[cfg(test)]
#[path = "bildkette_w4_mf_task_tests.rs"]
mod bildkette_w4_mf_task_tests;
#[cfg(test)]
#[path = "bildkette_w4_task_tests.rs"]
mod bildkette_w4_task_tests;
// Merge W2: layouts and HEVC/AV1 views shared with Linux (`nvenc_shared`).
use crate::screen::os::nvenc_shared::codecs as nvenc_codecs;
#[path = "nvenc_device.rs"]
mod nvenc_device;
#[path = "nvenc_encode.rs"]
mod nvenc_encode;
#[path = "nvenc_ffi.rs"]
mod nvenc_ffi;
#[path = "vendor_encode.rs"]
mod vendor_encode;

/// PCI vendor ids of `DXGI_ADAPTER_DESC::VendorId` (assumption A-PCI:
/// NVIDIA 0x10DE, AMD 0x1002, values of the PCI ID repository).
pub(super) const VENDOR_NVIDIA: u32 = 0x10de;
pub(super) const VENDOR_AMD: u32 = 0x1002;

/// The codecs of the vendor interfaces in the order of `Found`.
pub(super) const CODECS: [CodecKind; 3] = [CodecKind::H264, CodecKind::Hevc, CodecKind::Av1];

/// The two vendor interfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Vendor {
    Nvenc,
    Amf,
}

impl Vendor {
    /// The factory name of `codec` (`nvenc`, `nvenc-hevc`, `amf-av1`, …; the
    /// host process's `refused` list keeps the codecs apart, as BK6 names
    /// the Linux NVENC factories).
    pub(super) fn name(self, codec: CodecKind) -> String {
        match (self, codec) {
            (Self::Nvenc, codec) => nvenc_codecs::factory_name(codec),
            (Self::Amf, CodecKind::H264) => amf_encode::AMF_NAME.into(),
            (Self::Amf, codec) => format!("{}-{}", amf_encode::AMF_NAME, codec.key()),
        }
    }

    pub(super) fn pci_vendor(self) -> u32 {
        match self {
            Self::Nvenc => VENDOR_NVIDIA,
            Self::Amf => VENDOR_AMD,
        }
    }
}

/// What the probe of this process found: the largest picture per
/// interface and codec (`CODECS` order: H.264, HEVC, AV1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Found {
    pub nvenc: [Option<FrameSize>; 3],
    pub amf: [Option<FrameSize>; 3],
}

/// The encoders of plan 4.10a: the vendor interfaces this process found
/// before `media_foundation` (the MF factories in their order: hardware
/// MFTs, then the software MFT).
pub(super) fn vendor_encoders(
    media_foundation: Vec<Arc<dyn EncoderFactory>>,
) -> Vec<Arc<dyn EncoderFactory>> {
    ranked(found(), media_foundation)
}

/// The rank for `found` (pure, for the task tests): NVENC, then AMF, each
/// with H.264, HEVC and AV1 where found, then Media Foundation.
pub(super) fn ranked(
    found: Found,
    media_foundation: Vec<Arc<dyn EncoderFactory>>,
) -> Vec<Arc<dyn EncoderFactory>> {
    let mut encoders: Vec<Arc<dyn EncoderFactory>> = Vec::new();
    for (vendor, sizes) in [(Vendor::Nvenc, found.nvenc), (Vendor::Amf, found.amf)] {
        for (codec, max) in CODECS.into_iter().zip(sizes) {
            if let Some(max) = max {
                encoders.push(Arc::new(VendorFactory {
                    vendor,
                    codec,
                    max,
                    refused: OnceLock::new(),
                }));
            }
        }
    }
    encoders.extend(media_foundation);
    encoders
}

fn found() -> Found {
    static FOUND: OnceLock<Found> = OnceLock::new();
    *FOUND.get_or_init(|| Found {
        nvenc: nvenc_ffi::load()
            .ok()
            .and_then(|_| adapter_device(VENDOR_NVIDIA))
            .map_or([None; 3], |device| nvenc_device::probe(&device)),
        amf: amf_encode::present()
            .then(|| adapter_device(VENDOR_AMD))
            .flatten()
            .map_or([None; 3], |device| {
                CODECS.map(|codec| amf_encode::probe(&device, codec))
            }),
    })
}

/// A D3D11 device on the first hardware adapter of `vendor` (one with a
/// desktop output first, `dxgi_adapters::adapters`).
fn adapter_device(vendor: u32) -> Option<ID3D11Device> {
    let adapter = dxgi_adapters::of_vendor(vendor)?;
    let adapter = adapter.adapter.cast::<IDXGIAdapter>().ok()?;
    super::d3d_device::create_on(Some(&adapter))
        .ok()
        .map(|set| set.device)
}

/// Plan 4.10a with P3 (b): whether the capture's adapter has an encoder of
/// `codec` that takes its textures (its vendor interface or a hardware
/// MFT); a vendor encoder of another adapter then gives way to it instead
/// of opening on a device of its own (the iGPU of a hybrid laptop).
fn capture_adapter_encodes(codec: CodecKind) -> bool {
    let Some(capture) = d3d_input::capture_adapter() else {
        return false;
    };
    let found = found();
    let index = CODECS.iter().position(|own| *own == codec);
    let vendor = match (dxgi_adapters::vendor_of_luid(capture), index) {
        (Some(VENDOR_NVIDIA), Some(index)) => found.nvenc[index].is_some(),
        (Some(VENDOR_AMD), Some(index)) => found.amf[index].is_some(),
        _ => false,
    };
    vendor || super::mf_common::hardware_mft_on(codec, capture)
}

/// The PCI vendor id of the adapter `device` runs on.
pub(super) fn vendor_of(device: &ID3D11Device) -> Option<u32> {
    let dxgi: IDXGIDevice = device.cast().ok()?;
    // SAFETY: plain queries of the live DXGI device and its adapter.
    let desc = unsafe { dxgi.GetAdapter().and_then(|adapter| adapter.GetDesc()) }.ok()?;
    Some(desc.VendorId)
}

/// The smallest side of an HEVC or AV1 picture the vendor encoders are
/// opened with (assumption A-BK6-NV-MIN, as BK6's Linux NVENC factory).
pub(super) const VIDEO_MIN_SIDE: u32 = 64;

/// Whether a vendor encoder of `codec` and largest picture `max` takes
/// `choice` at `size` (pure, for the task tests): H.264 High only, HEVC and
/// AV1 4:2:0 Main only (no profile in the choice).
pub(super) fn check_request(
    codec: CodecKind,
    max: FrameSize,
    choice: &CodecChoice,
    size: FrameSize,
) -> io::Result<()> {
    let unsupported = |text: String| io::Error::new(io::ErrorKind::Unsupported, text);
    let (profile, min) = match codec {
        CodecKind::H264 => (Some(H264Profile::High), 16),
        CodecKind::Hevc | CodecKind::Av1 => (None, VIDEO_MIN_SIDE),
        CodecKind::Tiles => return Err(unsupported("vendor encoders have no tiles".into())),
    };
    if choice.codec != codec || choice.profile != profile {
        return Err(unsupported(format!(
            "vendor encoder of {} offers {} only",
            codec.key(),
            codec.profile_name(profile)
        )));
    }
    FrameLimits::RESOURCE.check(size)?;
    let fits = size.width <= max.width && size.height <= max.height;
    if size != size.even() || !fits || size.width < min || size.height < min {
        return Err(unsupported(format!(
            "vendor encoder cannot encode {}x{}",
            size.width, size.height
        )));
    }
    if size.pixels() > choice.max_pixels {
        return Err(unsupported("frame exceeds the codec pixel limit".into()));
    }
    Ok(())
}

/// One vendor interface and codec in the codec register.
pub(super) struct VendorFactory {
    pub vendor: Vendor,
    pub codec: CodecKind,
    pub max: FrameSize,
    /// Settings the encoder refused in the probe (B-21), once per process.
    pub refused: OnceLock<Vec<String>>,
}

impl VendorFactory {
    /// The capture's device when it is the vendor's GPU, else (P3 (b)) a
    /// device of its own on the vendor's adapter when the capture runs on
    /// none or another one; without admission always one of its own.
    fn input(&self, size: FrameSize, name: &str, admitted: bool) -> io::Result<GpuInput> {
        let pci = self.vendor.pci_vendor();
        if !admitted {
            return d3d_input::own_for_vendor(pci, size);
        }
        match d3d_input::attach_device(size, |device| vendor_of(device) == Some(pci)) {
            Ok(gpu) => Ok(gpu),
            Err(reason) => match d3d_input::input_route(Err(reason), true) {
                InputRoute::OwnDevice if !capture_adapter_encodes(self.codec) => {
                    d3d_input::own_for_vendor(pci, size)
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("{name}: not on the capture's device ({reason:?})"),
                )),
            },
        }
    }

    fn create(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: Option<CaptureGrant>,
        bps: u64,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        check_request(self.codec, self.max, choice, size)?;
        let name = self.vendor.name(self.codec);
        let gpu = self.input(size, &name, grant.is_some())?;
        let device = gpu.device().clone();
        let (fps, codec) = (fps.max(1), self.codec);
        // r1: no fixed ceiling; the pairing's, then the session's own.
        let ceiling = crate::screen::core::rate_control::codec_ceiling_bps(choice, size, fps);
        let encoder: Box<dyn ScreenEncoder> = match self.vendor {
            Vendor::Nvenc => {
                let session = nvenc_encode::NvencSession::open_at(&device, codec, size, fps, bps)?;
                let hull = match grant {
                    Some(grant) => {
                        vendor_encode::VendorEncoder::new(session, &name, codec, size, grant, gpu)
                    }
                    None => {
                        vendor_encode::VendorEncoder::plain(session, &name, codec, size, fps, gpu)
                    }
                };
                Box::new(hull.with_ceiling(ceiling)) as Box<dyn ScreenEncoder>
            }
            Vendor::Amf => {
                let session = amf_encode::AmfSession::open_at(&device, codec, size, fps, bps)?;
                let hull = match grant {
                    Some(grant) => {
                        vendor_encode::VendorEncoder::new(session, &name, codec, size, grant, gpu)
                    }
                    None => {
                        vendor_encode::VendorEncoder::plain(session, &name, codec, size, fps, gpu)
                    }
                };
                Box::new(hull.with_ceiling(ceiling)) as Box<dyn ScreenEncoder>
            }
        };
        Ok(encoder)
    }
}

impl EncoderFactory for VendorFactory {
    fn name(&self) -> String {
        self.vendor.name(self.codec)
    }

    fn offers(&self) -> Vec<CodecOffer> {
        let offer = match self.codec {
            CodecKind::H264 => CodecOffer::h264(H264Profile::High, true),
            codec => CodecOffer::video(codec, false, true),
        };
        vec![offer.with_max(self.max)]
    }

    /// AMF: the properties its component refused in a probe session (they
    /// stay at the encoder's default, B-21); NVENC takes its configuration
    /// whole or refuses the session.
    fn refused_settings(&self) -> Vec<String> {
        self.refused
            .get_or_init(|| match self.vendor {
                Vendor::Amf => adapter_device(VENDOR_AMD)
                    .map(|device| amf_encode::refused_settings(&device, self.codec))
                    .unwrap_or_default(),
                Vendor::Nvenc => Vec::new(),
            })
            .clone()
    }

    /// The vendor's adapter (P3: the key of its verdicts).
    fn device(&self) -> String {
        dxgi_adapters::identity_of_vendor(self.vendor.pci_vendor())
    }

    /// P3 (b): without admission (the run-time check, `bench --local`) on a
    /// device of its own, BGRA pictures copied up.
    fn open(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let start = crate::screen::core::rate_control::START_DIRECT_BPS;
        self.create(choice, size, fps, None, start)
    }

    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let start = crate::screen::core::rate_control::START_DIRECT_BPS;
        self.create(choice, size, fps, Some(grant), start)
    }

    /// The session opens at the rate the session gives it (plan 2026-11-17).
    fn open_admitted_at(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        grant: CaptureGrant,
        bps: u64,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.create(choice, size, fps, Some(grant), bps)
    }
}
