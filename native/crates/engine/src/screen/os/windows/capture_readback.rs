//! Grant-first staging and CPU readback. Tail bands reuse the same native
//! staging capacity; no source-sized temporary copy is used for a scaled frame.
use super::capture_slots::CaptureBudget;
use super::d3d_device::DeviceSet;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_image::{AdmittedBytes, CaptureImageBuilder};
use crate::screen::core::pixel_convert::FrameSize;
use std::io;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

struct Staging {
    texture: ID3D11Texture2D,
    size: FrameSize,
    // Texture COM ownership ends before its requested pixel promise.
    _slice: CaptureSlice,
}
#[derive(Default)]
pub(super) struct Readback {
    staging: Option<Staging>,
}
struct Sums {
    values: Vec<[u64; 4]>,
    _slice: CaptureSlice,
}
impl Sums {
    fn new(width: u32, grant: &CaptureGrant) -> io::Result<Self> {
        let bytes = u64::from(width)
            .checked_mul(std::mem::size_of::<[u64; 4]>() as u64)
            .ok_or_else(|| io::Error::other("capture filter storage overflow"))?;
        let slice = grant.claim(CaptureOwner::FilterScratch, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(width as usize)
            .map_err(|_| io::Error::other("capture filter allocation failed"))?;
        values.resize(width as usize, [0; 4]);
        Ok(Self {
            values,
            _slice: slice,
        })
    }
}
impl Readback {
    pub(super) fn reset(&mut self) {
        self.staging = None;
    }
    fn staging_for(
        &mut self,
        devices: &DeviceSet,
        size: FrameSize,
        grant: &CaptureGrant,
    ) -> io::Result<ID3D11Texture2D> {
        devices.frame_limits().check(size)?;
        if self.staging.as_ref().map(|s| s.size) != Some(size) {
            // Never retain a stale staging allocation during replacement.
            self.reset();
            let bytes = crate::screen::core::capture_plan::bgra_bytes(size)?;
            let slice = grant.claim(CaptureOwner::Readback, bytes)?;
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
            let mut texture = None;
            // SAFETY: checked dimensions and a live out pointer. The slice
            // was acquired before CreateTexture2D can allocate its surface.
            unsafe {
                devices
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }
            .map_err(io::Error::other)?;
            self.staging = Some(Staging {
                texture: texture.ok_or_else(|| io::Error::other("no staging texture"))?,
                size,
                _slice: slice,
            });
        }
        self.staging
            .as_ref()
            .map(|s| s.texture.clone())
            .ok_or_else(|| io::Error::other("no staging texture"))
    }
    pub(super) fn read(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        width: u32,
        height: u32,
        budget: &CaptureBudget,
    ) -> io::Result<CaptureImageBuilder> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: valid texture and out structure.
        unsafe { source.GetDesc(&mut desc) };
        let native = FrameSize::new(width, height);
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM || desc.SampleDesc.Count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "capture texture is not single-sample BGRA8",
            ));
        }
        if native != budget.geometry().native || width > desc.Width || height > desc.Height {
            return Err(io::Error::other(
                "capture content differs from admitted native geometry",
            ));
        }
        let geometry = budget.geometry();
        let grant = budget.grant()?;
        let staging = self.staging_for(devices, geometry.staging(), grant)?;
        let mut out = CaptureImageBuilder::new(
            geometry.delivered,
            geometry.limits,
            grant,
            CaptureOwner::SourcePixels,
        )?;
        if geometry.delivered != native {
            let band_bytes = crate::screen::core::capture_plan::bgra_bytes(geometry.staging())?;
            let mut band = AdmittedBytes::new(grant, CaptureOwner::Readback, band_bytes)?;
            let mut sums = Sums::new(geometry.delivered.width, grant)?;
            for y in 0..geometry.delivered.height {
                sums.values.fill([0; 4]);
                let (top, bottom) = span(y, geometry.delivered.height, native.height);
                let mut row = top;
                while row < bottom {
                    let rows = (bottom - row).min(geometry.staging().height);
                    copy_band(devices, source, &staging, native.width, row, rows);
                    let size = FrameSize::new(native.width, rows);
                    let length = size.checked_bgra_len()?;
                    map_into(devices, &staging, size, &mut band.as_mut_slice()[..length])?;
                    for line in band.as_slice()[..length].chunks_exact(native.width as usize * 4) {
                        for (x, sum) in sums.values.iter_mut().enumerate() {
                            let (left, right) =
                                span(x as u32, geometry.delivered.width, native.width);
                            for pixel in line[left as usize * 4..right as usize * 4].chunks_exact(4)
                            {
                                for (sum, value) in sum.iter_mut().zip(pixel) {
                                    *sum += u64::from(*value);
                                }
                            }
                        }
                    }
                    row += rows;
                }
                let target = out
                    .row_mut(y)
                    .ok_or_else(|| io::Error::other("capture target row missing"))?;
                for (x, (pixel, sum)) in target.chunks_exact_mut(4).zip(&sums.values).enumerate() {
                    let (left, right) = span(x as u32, geometry.delivered.width, native.width);
                    let count = u64::from(bottom - top) * u64::from(right - left);
                    for (value, sum) in pixel.iter_mut().zip(sum) {
                        *value = ((*sum + count / 2) / count) as u8;
                    }
                }
            }
        } else {
            copy_band(devices, source, &staging, width, 0, height);
            map_into(devices, &staging, native, out.pixels_mut())?;
        }
        Ok(out)
    }
}
fn span(index: u32, target: u32, source: u32) -> (u32, u32) {
    let first = u64::from(index) * u64::from(source) / u64::from(target);
    let end = u64::from(index + 1) * u64::from(source) / u64::from(target);
    (first as u32, end.max(first + 1) as u32)
}
fn copy_band(
    devices: &DeviceSet,
    source: &ID3D11Texture2D,
    staging: &ID3D11Texture2D,
    width: u32,
    top: u32,
    rows: u32,
) {
    let region = D3D11_BOX {
        left: 0,
        top,
        front: 0,
        right: width,
        bottom: top + rows,
        back: 1,
    };
    // SAFETY: read() checked the whole source; bands remain inside it and
    // the fixed staging capacity. No texture allocation occurs here.
    unsafe {
        devices
            .context
            .CopySubresourceRegion(staging, 0, 0, 0, 0, source, 0, Some(&region));
    }
}
fn map_into(
    devices: &DeviceSet,
    texture: &ID3D11Texture2D,
    size: FrameSize,
    target: &mut [u8],
) -> io::Result<()> {
    let row = (size.width as usize)
        .checked_mul(4)
        .ok_or_else(|| io::Error::other("capture row overflow"))?;
    if target.len() != size.checked_bgra_len()? {
        return Err(io::Error::other("capture target length mismatch"));
    }
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: staging texture has CPU read access.
    unsafe {
        devices
            .context
            .Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    }
    .map_err(io::Error::other)?;
    let pitch = mapped.RowPitch as usize;
    let total = pitch
        .checked_mul(size.height as usize - 1)
        .and_then(|n| n.checked_add(row));
    let result = match total
        .filter(|n| *n <= isize::MAX as usize && pitch >= row && !mapped.pData.is_null())
    {
        Some(total) => {
            // SAFETY: Map supplies RowPitch for every requested staging row;
            // checked arithmetic validates the addressed span. Padding is
            // neither copied nor claimed as a fixed product allocation.
            let rows = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
            for (line, out) in target.chunks_exact_mut(row).enumerate() {
                out.copy_from_slice(&rows[line * pitch..line * pitch + row]);
                for pixel in out.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
            Ok(())
        }
        None => Err(io::Error::other(
            "mapped capture texture has unusable row pitch",
        )),
    };
    // SAFETY: Map succeeded, including when row validation failed.
    unsafe { devices.context.Unmap(texture, 0) };
    result
}
