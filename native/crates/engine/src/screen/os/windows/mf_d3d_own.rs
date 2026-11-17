//! A Direct3D 11 device of the encoder's own (plan 2026-11-17
//! „Windows-Bild“ P3 (b), TODO `VERB-WIN-KODIERER`). Measured on
//! 2026-11-17: on a VM host the capture ran on the virtual display adapter
//! and NVENC, listed first, never opened; the hardware MFT took NV12 in
//! system memory. NVENC, AMF and a D3D11-aware hardware MFT whose capture
//! runs on no adapter yet (`NoCapture`: a check, `bench --local`) or on
//! another one (`OtherAdapter`, `OtherVendor`) now get a device of their own
//! on their adapter, and every picture is converted on the CPU and copied
//! up (`GpuInput::texture` for an admitted picture, `upload_bgra` for one
//! without admission). Such an input is never armed: the capture of this
//! process keeps its own route (the CPU route, as nothing armed it) and the
//! bridge of `mf_d3d_input.rs` is not touched.
use std::io;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_CPU_ACCESS_WRITE};
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIDeviceManager, IMFSample, IMFTransform, MF_SA_D3D11_AWARE,
};

use super::super::video_convert::nv12_texture;
use super::{surface, GpuInput, SystemReason};
use crate::screen::core::pixel_convert::{BgraView, FrameSize};
use crate::screen::os::d3d_device::{create_on, DeviceSet};
use crate::screen::os::dxgi_adapters;
use crate::screen::os::mf_capture_media::upload;
use crate::screen::os::mf_common::adapter_of;

/// Where an encoder's pictures come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum InputRoute {
    /// Textures of the capture's device (BK2a, BK12).
    Capture,
    /// A device of its own on the encoder's adapter, pictures copied up.
    OwnDevice,
    /// NV12 in system memory (a vendor encoder refuses instead).
    SystemMemory,
}

/// The route of an encoder at its opening (pure, for the task tests): the
/// capture's textures when it armed; a device of its own when the capture
/// runs on no or another adapter and the encoder takes D3D11 textures
/// (`aware`); otherwise system memory, as before (a GPU path switched off
/// after a failure, a capture device without video support, a refused
/// device manager).
pub(in crate::screen::os) fn input_route(
    attached: Result<(), SystemReason>,
    aware: bool,
) -> InputRoute {
    match attached {
        Ok(()) => InputRoute::Capture,
        Err(SystemReason::NoCapture | SystemReason::OtherAdapter | SystemReason::OtherVendor)
            if aware =>
        {
            InputRoute::OwnDevice
        }
        Err(_) => InputRoute::SystemMemory,
    }
}

/// A device on the adapter of the packed LUID `luid`, else on the first
/// hardware adapter of the PCI `vendor`.
fn own_device(luid: Option<u64>, vendor: Option<u32>) -> io::Result<DeviceSet> {
    let adapters = dxgi_adapters::adapters();
    let found = luid
        .and_then(|luid| {
            adapters
                .iter()
                .find(|adapter| dxgi_adapters::packed(adapter.luid) == luid)
        })
        .or_else(|| {
            vendor.and_then(|vendor| adapters.iter().find(|adapter| adapter.vendor == vendor))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no adapter of the encoder"))?;
    let adapter: IDXGIAdapter = found.adapter.cast().map_err(io::Error::other)?;
    create_on(Some(&adapter)).map_err(io::Error::other)
}

impl GpuInput {
    /// An input on `devices`, never armed; `manager` is the device manager
    /// handed to an MFT (none for NVENC and AMF).
    fn own(devices: DeviceSet, manager: Option<IMFDXGIDeviceManager>, size: FrameSize) -> Self {
        Self {
            raw: devices.device.as_raw() as usize,
            device: devices.device,
            context: devices.context,
            _manager: manager,
            size,
            token: None,
            target: None,
            upload: None,
            plain: None,
            scale: None,
            in_flight: None,
        }
    }

    /// Whether this input runs on a device of its own (P3 (b)).
    pub(in crate::screen::os) fn is_own(&self) -> bool {
        self.token.is_none()
    }

    /// A picture without admission (a check, `bench --local`) in the
    /// encoder's texture: converted on all cores and copied up.
    pub(in crate::screen::os) fn upload_bgra(
        &mut self,
        view: &BgraView,
    ) -> io::Result<ID3D11Texture2D> {
        if view.size() != self.size {
            return Err(io::Error::other("picture size differs from the encoder"));
        }
        if self.plain.is_none() {
            let target = nv12_texture(&self.device, self.size, None)?;
            let staging = nv12_texture(&self.device, self.size, Some(D3D11_CPU_ACCESS_WRITE))?;
            self.plain = Some((target, staging));
        }
        let (target, staging) = self
            .plain
            .clone()
            .ok_or_else(|| io::Error::other("no input texture"))?;
        upload(&self.context, view, &staging, &target)?;
        Ok(target)
    }

    /// The input sample of `view` for an MFT (`upload_bgra` in a DXGI
    /// buffer).
    pub(in crate::screen::os) fn sample_bgra(
        &mut self,
        view: &BgraView,
        time: i64,
        duration: i64,
    ) -> io::Result<IMFSample> {
        let texture = self.upload_bgra(view)?;
        surface::surface_sample(&texture, time, duration)
    }
}

/// P3 (b) for a hardware MFT: a device of its own on the MFT's adapter
/// (`adapter` from the enumeration, else the transform's own LUID), handed
/// to `transform` before its media types; why not otherwise.
pub(in crate::screen::os) fn attach_own(
    transform: &IMFTransform,
    size: FrameSize,
    adapter: Option<u64>,
) -> Result<GpuInput, SystemReason> {
    // SAFETY: attribute reads on the activated transform.
    let attributes = unsafe { transform.GetAttributes() }.ok();
    let aware = attributes
        .as_ref()
        .and_then(|a| unsafe { a.GetUINT32(&MF_SA_D3D11_AWARE) }.ok())
        .is_some_and(|value| value != 0);
    if !aware {
        return Err(SystemReason::NotD3D11Aware);
    }
    let adapter = adapter.or_else(|| attributes.as_ref().and_then(adapter_of));
    let devices = own_device(adapter, None).map_err(|error| {
        eprintln!("screen encoder: no device of its own: {error}");
        SystemReason::NoVideo
    })?;
    if !devices.has_video() {
        return Err(SystemReason::NoVideo);
    }
    let manager = surface::hand_manager(transform, &devices.device).map_err(|error| {
        eprintln!("screen encoder refused a device of its own: {error}");
        SystemReason::ManagerRefused
    })?;
    Ok(GpuInput::own(devices, Some(manager), size))
}

/// P3 (b) for NVENC and AMF: a device of its own on the first hardware
/// adapter of the PCI `vendor`.
pub(in crate::screen::os) fn own_for_vendor(vendor: u32, size: FrameSize) -> io::Result<GpuInput> {
    let devices = own_device(None, Some(vendor))?;
    Ok(GpuInput::own(devices, None, size))
}
