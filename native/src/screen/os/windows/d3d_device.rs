//! Direct3D 11 for the Windows capture (SCR-M4, decision U16-3): the device
//! of one source and the read-back of its pictures. There is no GPU path to
//! the encoder in this strand: every picture is copied into a staging
//! texture and from there row by row into a `BgraImage` (about 8.3 MB per
//! picture at 1920 × 1080), so the codec chain stays as it is (`downscale_bgra`,
//! the NV12 conversion of `mf_media`, `ScreenEncoder::encode(&BgraImage)`).
use std::io;

use windows::core::Interface;
use windows::Win32::Foundation::{E_POINTER, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

use crate::screen::core::pixel_convert::{downscale_bgra, BgraImage, BgraView, FrameSize};

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

/// The size a picture of `width` × `height` has after read-back: its own,
/// or the largest size within the frame limits when it is larger (a 5K
/// monitor); `BgraImage` and the host process frames stop at 4096 × 2304.
pub(super) fn delivered_size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width.max(1), height.max(1)).fit_within(FrameSize::MAX)
}

/// The staging texture of the read-back and its size.
#[derive(Default)]
pub(super) struct Readback {
    staging: Option<(ID3D11Texture2D, FrameSize)>,
}

impl Readback {
    /// Drops the staging texture (the source changed its size).
    pub(super) fn reset(&mut self) {
        self.staging = None;
    }

    fn staging_for(
        &mut self,
        devices: &DeviceSet,
        size: FrameSize,
    ) -> io::Result<&ID3D11Texture2D> {
        if self.staging.as_ref().map(|(_, have)| *have) != Some(size) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.width,
                Height: size.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut texture: Option<ID3D11Texture2D> = None;
            // SAFETY: `desc` and the out pointer outlive the call.
            unsafe {
                devices
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
                    .map_err(io::Error::other)?;
            }
            let texture =
                texture.ok_or_else(|| io::Error::other("no staging texture was created"))?;
            self.staging = Some((texture, size));
        }
        self.staging
            .as_ref()
            .map(|(texture, _)| texture)
            .ok_or_else(|| io::Error::other("no staging texture"))
    }

    /// The top-left `width` × `height` pixels of `source` (a texture of the
    /// format B8G8R8A8) as a picture with alpha 255: pixel transparency of
    /// a window must not reach the encoders.
    pub(super) fn read(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> io::Result<BgraImage> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `desc` is a valid out structure.
        unsafe { source.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM || desc.SampleDesc.Count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the capture texture is not an 8-bit BGRA texture",
            ));
        }
        let size = FrameSize::new(width.min(desc.Width), height.min(desc.Height));
        if size.width == 0 || size.height == 0 {
            return Err(io::Error::other("the capture texture is empty"));
        }
        let staging = self.staging_for(devices, size)?.clone();
        // SAFETY: both textures belong to this device; the box lies inside
        // the source.
        unsafe {
            if size.width == desc.Width && size.height == desc.Height {
                devices.context.CopyResource(&staging, source);
            } else {
                let region = D3D11_BOX {
                    left: 0,
                    top: 0,
                    front: 0,
                    right: size.width,
                    bottom: size.height,
                    back: 1,
                };
                devices.context.CopySubresourceRegion(
                    &staging,
                    0,
                    0,
                    0,
                    0,
                    source,
                    0,
                    Some(&region),
                );
            }
        }
        let tight = self.map_rows(devices, &staging, size)?;
        if size.is_valid_frame() {
            return BgraImage::from_vec(size, tight);
        }
        let view = BgraView::new(size, size.width as usize * 4, &tight)?;
        downscale_bgra(&view, delivered_size(size.width, size.height))
    }

    /// Maps `staging` and copies its rows into one tight buffer.
    fn map_rows(
        &self,
        devices: &DeviceSet,
        staging: &ID3D11Texture2D,
        size: FrameSize,
    ) -> io::Result<Vec<u8>> {
        let row = size.width as usize * 4;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture was created with CPU read access.
        unsafe {
            devices
                .context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(io::Error::other)?;
        }
        let pitch = mapped.RowPitch as usize;
        let mut tight = vec![0u8; row * size.height as usize];
        if pitch >= row && !mapped.pData.is_null() {
            let total = pitch * (size.height as usize - 1) + row;
            // SAFETY: a mapped staging texture holds `RowPitch` bytes per
            // row for `Height` rows, the last row at least `row` bytes.
            let mapped_rows =
                unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
            for (line, target) in tight.chunks_exact_mut(row).enumerate() {
                target.copy_from_slice(&mapped_rows[line * pitch..line * pitch + row]);
                for pixel in target.chunks_exact_mut(4) {
                    pixel[3] = 0xFF;
                }
            }
        }
        // SAFETY: the subresource was mapped above.
        unsafe { devices.context.Unmap(staging, 0) };
        if pitch < row || mapped.pData.is_null() {
            return Err(io::Error::other("the mapped capture texture is unusable"));
        }
        Ok(tight)
    }
}
