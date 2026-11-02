//! Direct3D 11 for the Windows capture (SCR-M4, decision U16-3): the device
//! of one source and the read-back of its pictures. There is no GPU path to
//! the encoder in this strand: every picture is copied into a staging
//! texture and from there into immutable grant-owned capture storage.
//! Readback and band resampling live in `capture_readback`; encoders borrow
//! the compatible `BgraImage` view without copying its pixels.
use std::io;

use windows::core::Interface;
use windows::Win32::Foundation::{E_POINTER, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

pub(super) use super::capture_readback::Readback;
use crate::screen::core::frame_budget::{FrameLimits, MAX_WORKING_BYTES};
use crate::screen::core::pixel_convert::FrameSize;

/// The Direct3D device of one source in both of its forms.
pub(super) struct DeviceSet {
    pub(super) device: ID3D11Device,
    pub(super) context: ID3D11DeviceContext,
    pub(super) winrt: windows::Graphics::DirectX::Direct3D11::IDirect3DDevice,
}

// SAFETY: a Direct3D 11 device is free-threaded and its immediate context is
// switched to multithread protection in `create_on`; the WinRT device wraps
// the same object. A source with its set moves between threads but one
// thread at a time uses it.
unsafe impl Send for DeviceSet {}

fn created(
    adapter: Option<&IDXGIAdapter>,
    driver: D3D_DRIVER_TYPE,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: the out pointers address locals that live through the call;
    // the adapter is a live interface or none.
    unsafe {
        D3D11CreateDevice(
            adapter,
            driver,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::from(E_POINTER)),
    }
}

/// A device on the default adapter: hardware, and when that fails WARP.
pub(super) fn create() -> windows::core::Result<DeviceSet> {
    create_on(None)
}

/// A device on `adapter` (the adapter that owns the output a duplication
/// needs), or on the default adapter.
pub(super) fn create_on(adapter: Option<&IDXGIAdapter>) -> windows::core::Result<DeviceSet> {
    let (device, context) = match adapter {
        Some(adapter) => created(Some(adapter), D3D_DRIVER_TYPE_UNKNOWN)?,
        None => created(None, D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|_| created(None, D3D_DRIVER_TYPE_WARP))?,
    };
    // Windows Graphics Capture uses the device from its own threads.
    if let Ok(protect) = context.cast::<ID3D11Multithread>() {
        // SAFETY: a live interface; the previous setting is of no interest.
        let _ = unsafe { protect.SetMultithreadProtected(true) };
    }
    let dxgi: IDXGIDevice = device.cast()?;
    // SAFETY: a live DXGI device of the device just created.
    let wrapped = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
    let winrt = wrapped.cast()?;
    Ok(DeviceSet {
        device,
        context,
        winrt,
    })
}

impl DeviceSet {
    pub(super) fn frame_limits(&self) -> FrameLimits {
        // SAFETY: immutable query on this live D3D11 device.
        let side = if unsafe { self.device.GetFeatureLevel() }.0 >= D3D_FEATURE_LEVEL_11_0.0 {
            16384
        } else {
            8192
        };
        FrameLimits {
            max_size: FrameSize::new(side, side),
            ..FrameLimits::RESOURCE
        }
    }
    pub(super) fn capture_limits(&self, native: FrameSize) -> io::Result<FrameLimits> {
        let mut limits = self.frame_limits();
        let bytes = native.checked_bgra_len()? as u64;
        if native.width == 0
            || native.height == 0
            || native.width > limits.max_size.width
            || native.height > limits.max_size.height
        {
            return Err(io::Error::other(
                "capture exceeds this D3D11 device's texture dimensions",
            ));
        }
        // Two native capture buffers replace two of the eight working images.
        // When native exceeds a working image, six delivered images share the rest.
        if bytes > MAX_WORKING_BYTES / 8 {
            let remaining = MAX_WORKING_BYTES
                .checked_sub(
                    bytes
                        .checked_mul(2)
                        .ok_or_else(|| io::Error::other("capture bytes overflow"))?,
                )
                .ok_or_else(|| io::Error::other("native capture buffers exceed 512 MiB"))?;
            limits.max_pixels = limits.max_pixels.min(remaining / 6 / 4);
        }
        limits.check(limits.fit(native))?;
        Ok(limits)
    }
    pub(super) fn delivered_size(&self, width: u32, height: u32) -> io::Result<FrameSize> {
        let native = FrameSize::new(width, height);
        Ok(self.capture_limits(native)?.fit(native))
    }
}
