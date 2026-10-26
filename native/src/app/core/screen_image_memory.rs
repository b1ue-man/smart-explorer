//! The stable pane owns local copy roles; aliases never detach pixels from their claim.
use crate::screen::core::{
    frame_budget::FrameLimits,
    frame_memory::MemoryHold,
    image_admission::{ImageAdmission, ImageHold, ImageMemory, ImageOwnerLimits, ImageRole},
    pixel_convert::BgraImage,
};
use eframe::egui::{Color32, ColorImage};
use std::{io, ops::Deref, sync::Arc};

pub(super) const LOGIN_BYTES: u64 = 4 * 4096 * 2304;
#[derive(Clone)]
pub(super) enum ImageAccess {
    Contract(MemoryHold),
    Local(Arc<ImageAdmission>),
}
impl ImageAccess {
    pub(super) fn login() -> io::Result<Self> {
        let limits = owner_limits()?;
        crate::screen::os::image_memory::admit_images(limits).map(Self::Local)
    }
    pub(super) fn claim(&self, role: ImageRole, bytes: u64) -> io::Result<Option<ImageHold>> {
        match self {
            Self::Contract(memory) => Ok(memory
                .as_ref()
                .map(|lease| ImageMemory::from_contract(lease.clone()))),
            Self::Local(admission) => admission
                .try_claim(role, bytes)
                .map(ImageMemory::from_slice)
                .map(Some),
        }
    }
}
pub(super) fn owner_limits() -> io::Result<ImageOwnerLimits> {
    let scratch = super::screen_scaling::scratch_limit()?;
    let metadata = metadata_bytes();
    // Two raw N slots belong to ViewerSession. This pane supplies 3N+6D+S.
    let total = LOGIN_BYTES
        .checked_mul(9)
        .and_then(|n| n.checked_add(scratch))
        .and_then(|n| n.checked_add(metadata * 6))
        .ok_or_else(overflow)?;
    ImageOwnerLimits::new(total)?
        .with_role(ImageRole::ColorSource, LOGIN_BYTES, 3)?
        .with_role(ImageRole::ScaledOutput, LOGIN_BYTES, 3)?
        .with_role(ImageRole::FilterScratch, scratch, 1)?
        .with_role(ImageRole::UploadStaging, LOGIN_BYTES, 1)?
        .with_role(ImageRole::RenderSurface, LOGIN_BYTES, 2)?
        .with_role(ImageRole::ImageMetadata, metadata, 6)
}
fn metadata_bytes() -> u64 {
    (std::mem::size_of::<CpuPixels>() + std::mem::size_of::<ColorImage>()) as u64
}
pub(super) fn overflow() -> io::Error {
    io::Error::other("image allocation size overflow")
}
pub(super) fn pixel_bytes(size: [usize; 2]) -> io::Result<u64> {
    let count = size[0].checked_mul(size[1]).ok_or_else(overflow)?;
    u64::try_from(count)
        .ok()
        .and_then(|n| n.checked_mul(std::mem::size_of::<Color32>() as u64))
        .filter(|_| !size.contains(&0))
        .ok_or_else(overflow)
}
/// Fields are private: no caller can clone a naked egui Arc without its end anchor.
pub(crate) struct CpuPixels {
    image: Arc<ColorImage>,
    access: ImageAccess,
    _pixels: Option<ImageHold>,
    _metadata: Option<ImageHold>,
}
impl Deref for CpuPixels {
    type Target = ColorImage;
    fn deref(&self) -> &ColorImage {
        &self.image
    }
}
impl CpuPixels {
    pub(super) fn convert(image: &BgraImage, access: ImageAccess) -> io::Result<Arc<Self>> {
        let size = [image.size().width as usize, image.size().height as usize];
        let allocation = PixelAllocation::new(access, ImageRole::ColorSource, size)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(image.data().len() / 4)
            .map_err(|_| io::Error::other("RGBA allocation failed"))?;
        for p in image.data().chunks_exact(4) {
            pixels.push(Color32::from_rgb(p[2], p[1], p[0]));
        }
        Ok(allocation.finish(ColorImage { size, pixels }))
    }
    pub(super) fn access(&self) -> &ImageAccess {
        &self.access
    }
    pub(super) fn upload_claims(&self, surface: Option<ImageHold>) -> io::Result<GpuClaims> {
        let bytes = pixel_bytes(self.size)?;
        let staging = self.access.claim(ImageRole::UploadStaging, bytes)?;
        let surface = match surface {
            Some(surface) => Some(surface),
            None => self.access.claim(ImageRole::RenderSurface, bytes)?,
        };
        Ok(GpuClaims { surface, staging })
    }
    /// Pinned Glow set_texture consumes this delta synchronously and retains no CPU Arc.
    /// The borrow keeps this complete owner alive through that entire call.
    pub(super) fn upload_glow(
        &self,
        painter: &mut eframe::egui_glow::Painter,
        id: eframe::egui::TextureId,
        create: impl FnOnce() -> Result<
            eframe::egui_glow::glow::Texture,
            eframe::egui_glow::GlOperationError,
        >,
    ) -> Result<(), eframe::egui_glow::GlOperationError> {
        let delta = eframe::egui::epaint::ImageDelta::full(
            self.image.clone(),
            eframe::egui::TextureOptions::NEAREST,
        );
        painter.try_set_texture_with(id, &delta, create)
    }
}
pub(super) struct PixelAllocation {
    access: ImageAccess,
    pixels: Option<ImageHold>,
    metadata: Option<ImageHold>,
}
impl PixelAllocation {
    pub(super) fn new(access: ImageAccess, role: ImageRole, size: [usize; 2]) -> io::Result<Self> {
        let pixels = access.claim(role, pixel_bytes(size)?)?;
        let metadata = access.claim(ImageRole::ImageMetadata, metadata_bytes())?;
        Ok(Self {
            access,
            pixels,
            metadata,
        })
    }
    pub(super) fn finish(self, image: ColorImage) -> Arc<CpuPixels> {
        Arc::new(CpuPixels {
            image: Arc::new(image),
            access: self.access,
            _pixels: self.pixels,
            _metadata: self.metadata,
        })
    }
}
#[derive(Default)]
pub(super) struct GpuClaims {
    pub surface: Option<ImageHold>,
    pub staging: Option<ImageHold>,
}
pub(super) fn login_limits() -> FrameLimits {
    FrameLimits::LEGACY
}

#[cfg(test)]
#[path = "batches_login_image_task_tests.rs"]
mod tests;
