//! The pointer of a Desktop Duplication picture that stays on the GPU (plan
//! 2026-11-17 P9). Measured on v0.5.198: every Desktop Duplication picture
//! of a session whose viewer leaves the pointer in the picture (the default
//! of `se screen bench`, and of a viewer without the P6 overlay) was read
//! back to the CPU only to blend the pointer (`CpuReason::PointerBlended`),
//! converted there and copied up to the encoder again: `convert cpu`,
//! encode p50 16.8–17.9 ms where the same AMF encoder needs 3–5 ms.
//!
//! Now only the area under the pointer goes through the CPU: it is copied
//! into a small staging texture (at most the pointer's size, 512 x 512 by
//! `dxgi_pointer.rs`), mapped, blended with the CPU route's `blend` (the
//! same pixels as before), written into a small BGRA patch texture and drawn
//! by the video processor over the converted picture
//! (`VideoConverter::overlay`). The full picture never leaves the GPU.
//!
//! Both textures are claimed from the capture grant (`Readback`, beside the
//! NV12 ring) before they are created. Any failure ends this way for the
//! session: the pictures then take the CPU route with the pointer blended
//! there, as before, and `CpuReason::PointerBlended` names why.
use std::io;

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_route::pointer_patch;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize, Rect};
use crate::screen::os::capture_dxgi::pointer::{blend, PointerImage};
use crate::screen::os::d3d_device::DeviceSet;

/// The pointer to draw into one picture: its image and the top left of the
/// image on the output, in the picture's pixels.
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct PointerAt<'a> {
    pub image: &'a PointerImage,
    pub left: i32,
    pub top: i32,
}

/// A BGRA texture with its claim, released before the claim.
struct Owned {
    texture: ID3D11Texture2D,
    size: FrameSize,
    _claim: CaptureSlice,
}

/// The staging and patch textures of one capture.
#[derive(Default)]
pub(in crate::screen::os) struct PointerPatch {
    staging: Option<Owned>,
    patch: Option<Owned>,
    pixels: Vec<u8>,
    /// Why the GPU blend ended for this capture; `None` while it works.
    failed: Option<String>,
}

impl PointerPatch {
    /// Whether the pointer can be drawn on the GPU.
    pub(in crate::screen::os) fn usable(&self) -> bool {
        self.failed.is_none()
    }

    /// The textures go (the CPU route needs their budget for its staging
    /// texture, or the capture generation ends); a failure stays.
    pub(in crate::screen::os) fn release(&mut self) {
        self.staging = None;
        self.patch = None;
        self.pixels = Vec::new();
    }

    /// The GPU blend ends for this capture; the textures go.
    pub(in crate::screen::os) fn fail(&mut self, error: &io::Error) {
        if self.failed.is_none() {
            eprintln!(
                "screen capture: pointer on the GPU off, the CPU blends it from now on: {error}"
            );
            self.failed = Some(error.to_string());
        }
        self.release();
    }

    /// The patch for one picture of `native` size in `source`: the area
    /// under the pointer with the pointer blended in, and where it goes;
    /// `None` when the pointer lies outside the picture.
    pub(in crate::screen::os) fn prepare(
        &mut self,
        devices: &DeviceSet,
        source: &ID3D11Texture2D,
        native: FrameSize,
        pointer: PointerAt<'_>,
        grant: &CaptureGrant,
    ) -> io::Result<Option<(ID3D11Texture2D, Rect)>> {
        let image = pointer.image;
        let Some(at) = pointer_patch(native, pointer.left, pointer.top, image.width, image.height)
        else {
            return Ok(None);
        };
        let size = FrameSize::new(at.width, at.height);
        let staging = texture(devices, &mut self.staging, size, grant, true)?;
        let patch = texture(devices, &mut self.patch, size, grant, false)?;
        let length = size.checked_bgra_len()?;
        self.pixels.clear();
        self.pixels
            .try_reserve_exact(length)
            .map_err(|_| io::Error::other("pointer patch allocation failed"))?;
        self.pixels.resize(length, 0);
        let region = D3D11_BOX {
            left: at.x,
            top: at.y,
            front: 0,
            right: at.x + at.width,
            bottom: at.y + at.height,
            back: 1,
        };
        // SAFETY: `at` lies inside the source (`pointer_patch` clips it to
        // the even part of `native`) and inside the staging capacity.
        unsafe {
            devices
                .context
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, source, 0, Some(&region));
        }
        read_into(devices, &staging, size, &mut self.pixels)?;
        let (left, top) = (pointer.left - at.x as i32, pointer.top - at.y as i32);
        blend(
            &mut self.pixels,
            size,
            size,
            BgraRotation::Identity,
            image,
            left,
            top,
        );
        let target = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: at.width,
            bottom: at.height,
            back: 1,
        };
        // SAFETY: the box lies inside the patch texture; `pixels` holds its
        // tight rows (`width * 4` bytes each) through the call.
        unsafe {
            devices.context.UpdateSubresource(
                &patch,
                0,
                Some(&target),
                self.pixels.as_ptr().cast(),
                at.width * 4,
                0,
            );
        }
        Ok(Some((patch, at)))
    }
}

/// The texture in `slot` when it holds `size`, else a new one of the size
/// rounded up to 64 (fewer replacements while the pointer moves to an edge
/// and back), claimed from `grant` before it exists.
fn texture(
    devices: &DeviceSet,
    slot: &mut Option<Owned>,
    size: FrameSize,
    grant: &CaptureGrant,
    staging: bool,
) -> io::Result<ID3D11Texture2D> {
    let fits = |owned: &Owned| owned.size.width >= size.width && owned.size.height >= size.height;
    if let Some(owned) = slot.as_ref().filter(|owned| fits(owned)) {
        return Ok(owned.texture.clone());
    }
    *slot = None;
    let capacity = FrameSize::new(
        size.width.next_multiple_of(64),
        size.height.next_multiple_of(64),
    );
    devices.frame_limits().check(capacity)?;
    let claim = grant.claim(
        CaptureOwner::Readback,
        crate::screen::core::capture_plan::bgra_bytes(capacity)?,
    )?;
    let desc = |bind: u32| D3D11_TEXTURE2D_DESC {
        Width: capacity.width,
        Height: capacity.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: if staging {
            D3D11_USAGE_STAGING
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: bind,
        CPUAccessFlags: if staging {
            D3D11_CPU_ACCESS_READ.0 as u32
        } else {
            0
        },
        MiscFlags: 0,
    };
    let create = |desc: D3D11_TEXTURE2D_DESC| {
        let mut texture = None;
        // SAFETY: checked dimensions and a live out pointer; the claim was
        // taken before the texture can allocate.
        unsafe {
            devices
                .device
                .CreateTexture2D(&desc, None, Some(&mut texture))
        }
        .map_err(io::Error::other)?;
        texture.ok_or_else(|| io::Error::other("no pointer texture"))
    };
    let texture = if staging {
        create(desc(0))?
    } else {
        // A video processor input like the capture's own texture, else a
        // plain render target.
        let both = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
        create(desc(both)).or_else(|_| create(desc(D3D11_BIND_RENDER_TARGET.0 as u32)))?
    };
    *slot = Some(Owned {
        texture: texture.clone(),
        size: capacity,
        _claim: claim,
    });
    Ok(texture)
}

/// Copies the top left `size` of the mapped `staging` texture into the
/// tight rows of `out`, every pixel opaque (as the CPU route's `map_into`).
fn read_into(
    devices: &DeviceSet,
    staging: &ID3D11Texture2D,
    size: FrameSize,
    out: &mut [u8],
) -> io::Result<()> {
    let row = size.width as usize * 4;
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: a staging texture with CPU read access.
    unsafe {
        devices
            .context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    }
    .map_err(io::Error::other)?;
    let pitch = mapped.RowPitch as usize;
    let total = pitch
        .checked_mul(size.height as usize - 1)
        .and_then(|n| n.checked_add(row));
    let result = match total.filter(|_| pitch >= row && !mapped.pData.is_null()) {
        Some(total) if out.len() == row * size.height as usize => {
            // SAFETY: Map gives `RowPitch` bytes per row of the texture,
            // whose capacity holds `size`; the span was checked above.
            let rows = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
            for (line, target) in out.chunks_exact_mut(row).enumerate() {
                target.copy_from_slice(&rows[line * pitch..line * pitch + row]);
                for pixel in target.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
            Ok(())
        }
        _ => Err(io::Error::other("mapped pointer texture is unusable")),
    };
    // SAFETY: Map succeeded above.
    unsafe { devices.context.Unmap(staging, 0) };
    result
}
