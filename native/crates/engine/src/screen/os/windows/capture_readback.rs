//! Grant-first staging and CPU readback. Tail bands reuse the same native
//! staging capacity; no source-sized temporary copy is used for a scaled frame.
//!
//! Plan 2026-11-14 bildkette B-17 (package BK2a): while the encoder takes
//! textures, `gpu_picture` replaces the readback. The video processor
//! converts the capture texture into the NV12 ring before the frame is
//! released, and the pipeline gets a GPU picture that names the ring slot
//! (`capture_picture::GpuPicture` around `mf_capture_media::RingPicture`,
//! BK3b): no pixels in system memory, no CPU scaling. The ring takes the
//! `Readback` budget of the staging texture, which is freed first; the CPU
//! readback releases the ring first.
use super::capture_slots::{CaptureBudget, CaptureKind};
use super::d3d_device::DeviceSet;
use super::mf_capture_media::RingPicture;
use super::mf_encoder::d3d_input;
use super::mf_encoder::video_convert::Ring;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_image::{AdmittedBytes, CaptureImageBuilder};
use crate::screen::core::capture_picture::GpuPicture;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize};
use std::io;
use std::sync::Arc;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

/// Why a picture is read back to the CPU instead of staying on the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum CpuReason {
    /// No encoder of this process takes textures.
    NotArmed,
    /// The texture encoder runs on another device than this capture.
    OtherDevice,
    /// The GPU path failed earlier in this session.
    Disabled,
    /// The capture device has no video support.
    NoVideo,
    /// Desktop Duplication of a rotated output (the CPU rotates).
    Rotated,
    /// The readback scales to the capture limits.
    Scaled,
    /// The host blends the pointer into Desktop Duplication pictures.
    PointerBlended,
    /// A still image asked for this picture's exact BGRA (merge W1): one
    /// banded readback, the NV12 ring stays for the next pictures.
    Exact,
}

/// What the capture side knows when a picture arrives.
#[derive(Clone, Copy, Debug)]
pub(in crate::screen::os) struct CaptureFacts {
    pub armed: Option<usize>,
    pub device: usize,
    pub disabled: bool,
    pub video: bool,
    pub rotated: bool,
    pub scaled: bool,
    pub duplication: bool,
    /// The viewer draws the pointer itself (P6 overlay).
    pub overlay: bool,
}

/// Rank of 4.10a, conversion: on the GPU of the capture when the encoder
/// takes textures, else on the CPU.
pub(in crate::screen::os) fn capture_route(facts: CaptureFacts) -> Result<(), CpuReason> {
    match facts.armed {
        _ if facts.disabled => Err(CpuReason::Disabled),
        None => Err(CpuReason::NotArmed),
        Some(device) if device != facts.device => Err(CpuReason::OtherDevice),
        Some(_) if !facts.video => Err(CpuReason::NoVideo),
        Some(_) if facts.rotated => Err(CpuReason::Rotated),
        Some(_) if facts.scaled => Err(CpuReason::Scaled),
        Some(_) if facts.duplication && !facts.overlay => Err(CpuReason::PointerBlended),
        Some(_) => Ok(()),
    }
}

struct Staging {
    texture: ID3D11Texture2D,
    size: FrameSize,
    // Texture COM ownership ends before its requested pixel promise.
    _slice: CaptureSlice,
}
/// What a readback delivers: CPU pixels to finish (regions, pointer,
/// rotation), or a picture that stays on the GPU (BK3b).
pub(super) enum Read {
    Cpu(CaptureImageBuilder),
    Gpu(GpuPicture),
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
        // The ring belongs to this capture generation (GPU path).
        d3d_input::release_ring();
    }
    /// B-17: the picture goes into the NV12 ring on this device and the
    /// pipeline gets the GPU picture of its slot; else why the CPU reads it.
    fn gpu_picture(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        budget: &CaptureBudget,
    ) -> io::Result<Result<GpuPicture, CpuReason>> {
        let geometry = budget.geometry();
        let raw = devices.device.as_raw() as usize;
        let route = d3d_input::route_capture(devices, |armed, disabled, video| CaptureFacts {
            armed,
            device: raw,
            disabled,
            video,
            rotated: matches!(geometry.kind, CaptureKind::Dxgi(r) if r != BgraRotation::Identity),
            scaled: geometry.delivered != geometry.native,
            duplication: matches!(geometry.kind, CaptureKind::Dxgi(_)),
            overlay: crate::screen::core::cursor_overlay::worker().separated(),
        });
        if let Err(reason) = route {
            return Ok(Err(reason));
        }
        let grant = budget.grant()?;
        let identity = (grant.source().clone(), grant.generation());
        let staging = &mut self.staging;
        let found = d3d_input::ring_for(
            |ring| {
                ring.device == raw
                    && ring.source == identity.0
                    && ring.generation == identity.1
                    && ring.native == geometry.native
            },
            || *staging = None,
            || {
                Ring::create(
                    &devices.device,
                    &devices.context,
                    identity.clone(),
                    geometry.native,
                    |bytes| grant.claim(CaptureOwner::Readback, bytes),
                )
            },
        );
        let (ring, tag) = match found {
            Ok(found) => found,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(error),
            Err(error) => {
                d3d_input::disable(&format!("NV12 ring: {error}"));
                d3d_input::release_ring();
                return Ok(Err(CpuReason::Disabled));
            }
        };
        if let Err(error) = ring.write(source, tag) {
            d3d_input::disable(&format!("video processor: {error}"));
            drop(ring);
            d3d_input::release_ring();
            return Ok(Err(CpuReason::Disabled));
        }
        // Admitted by the capture's grant: the ring slots it names were
        // claimed from it (`Readback`), no system memory is needed.
        let surface = RingPicture {
            ring: Arc::downgrade(&ring),
            tag,
            size: ring.size,
        };
        let picture = GpuPicture::new(
            Arc::new(surface),
            geometry.delivered,
            geometry.limits,
            grant,
        )?;
        Ok(Ok(picture))
    }
    fn staging_for(
        &mut self,
        devices: &DeviceSet,
        size: FrameSize,
        grant: &CaptureGrant,
    ) -> io::Result<ID3D11Texture2D> {
        if self.staging.as_ref().map(|s| s.size) != Some(size) {
            // Never retain a stale staging allocation during replacement.
            self.staging = None;
            self.staging = Some(staging_texture(devices, size, grant)?);
        }
        self.staging
            .as_ref()
            .map(|s| s.texture.clone())
            .ok_or_else(|| io::Error::other("no staging texture"))
    }
    /// Merge W1, the exact still image on the GPU route: the BGRA source
    /// texture read back once, in bands, into a staging texture claimed
    /// from the `Readback` budget the NV12 ring leaves (the ring takes 3 of
    /// the 4 bytes per pixel the CPU route's staging texture had), freed
    /// right after. Each band goes through the CPU route's `copy_band` and
    /// `map_into`, so the pixels are bit-identical to the CPU route's (the
    /// GPU route has neither rotation nor scaling).
    fn exact(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        budget: &CaptureBudget,
    ) -> io::Result<CaptureImageBuilder> {
        let geometry = budget.geometry();
        let grant = budget.grant()?;
        let size = geometry.native;
        if geometry.delivered != size {
            return Err(io::Error::other("an exact still image is not scaled"));
        }
        // A staging texture of the CPU route frees its claim first.
        self.staging = None;
        let rows = exact_rows(size);
        let band = staging_texture(devices, FrameSize::new(size.width, rows), grant)?;
        let mut out =
            CaptureImageBuilder::new(size, geometry.limits, grant, CaptureOwner::SourcePixels)?;
        read_bands(devices, source, &band.texture, size, rows, out.pixels_mut())?;
        Ok(out)
    }
    pub(super) fn read(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        width: u32,
        height: u32,
        budget: &CaptureBudget,
    ) -> io::Result<Read> {
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
        match self.gpu_picture(devices, source, budget)? {
            Ok(picture) => return Ok(Read::Gpu(picture)),
            Err(CpuReason::Exact) => return self.exact(devices, source, budget).map(Read::Cpu),
            Err(_) => {}
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
        Ok(Read::Cpu(out))
    }
}
/// A BGRA staging texture of `size` the CPU reads, claimed from `grant`
/// (`Readback`) before it is created.
fn staging_texture(
    devices: &DeviceSet,
    size: FrameSize,
    grant: &CaptureGrant,
) -> io::Result<Staging> {
    devices.frame_limits().check(size)?;
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
    // SAFETY: checked dimensions and a live out pointer. The slice was
    // acquired before CreateTexture2D can allocate its surface.
    unsafe {
        devices
            .device
            .CreateTexture2D(&desc, None, Some(&mut texture))
    }
    .map_err(io::Error::other)?;
    Ok(Staging {
        texture: texture.ok_or_else(|| io::Error::other("no staging texture"))?,
        size,
        _slice: slice,
    })
}
/// Rows of one band of the exact readback: within the `Readback` budget the
/// NV12 ring leaves (a quarter of the picture), at most 64, at least one.
pub(super) fn exact_rows(size: FrameSize) -> u32 {
    (size.height / 4).clamp(1, 64)
}
/// `source` (BGRA of `size`) into `out` band by band through `staging` (at
/// least `rows` rows), each band with the CPU route's `copy_band` and
/// `map_into`.
fn read_bands(
    devices: &DeviceSet,
    source: &ID3D11Texture2D,
    staging: &ID3D11Texture2D,
    size: FrameSize,
    rows: u32,
    out: &mut [u8],
) -> io::Result<()> {
    let row = size.width as usize * 4;
    if out.len() != size.checked_bgra_len()? {
        return Err(io::Error::other("capture target length mismatch"));
    }
    for (top, count) in bands(size.height, rows) {
        copy_band(devices, source, staging, size.width, top, count);
        let part = FrameSize::new(size.width, count);
        let span = top as usize * row..(top + count) as usize * row;
        map_into(devices, staging, part, &mut out[span])?;
    }
    Ok(())
}
/// The bands `(top, rows)` that cover `height` rows in steps of `rows`.
pub(super) fn bands(height: u32, rows: u32) -> Vec<(u32, u32)> {
    let rows = rows.max(1);
    (0..height)
        .step_by(rows as usize)
        .map(|top| (top, rows.min(height - top)))
        .collect()
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

#[cfg(test)]
#[path = "bildkette_w1_still_task_tests.rs"]
mod bildkette_w1_still_task_tests;
