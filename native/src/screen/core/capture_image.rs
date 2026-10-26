//! Immutable shared capture pixels. A clone is an alias, never a pixel copy.
use super::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, CaptureSlice, SourceKey,
};
pub(crate) use super::capture_bytes::{AdmittedBytes, AdmittedRegions};
use super::frame_budget::FrameLimits;
use super::image_admission::{ImageAdmission, ImageRole};
use super::pixel_convert::{BgraImage, BgraRotation, BgraView, FrameSize};
use super::wire::invalid;
use std::io;
use std::sync::Arc;

pub(crate) struct CaptureImageBuilder {
    image: BgraImage,
    source: SourceKey,
    generation: CaptureGeneration,
    regions: Option<CaptureSlice>,
    pixels: CaptureSlice,
    grant: CaptureGrant,
}
struct PixelStorage {
    image: BgraImage,
    source: SourceKey,
    generation: CaptureGeneration,
    // Both allocations in image die before either corresponding promise.
    _regions: Option<CaptureSlice>,
    _pixels: CaptureSlice,
    grant: CaptureGrant,
}
#[derive(Clone)]
pub(crate) struct CapturedImage {
    storage: Arc<PixelStorage>,
}
impl std::fmt::Debug for CapturedImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.image().fmt(formatter)
    }
}
impl PartialEq for CapturedImage {
    fn eq(&self, other: &Self) -> bool {
        self.image() == other.image()
    }
}
impl Eq for CapturedImage {}
impl CaptureImageBuilder {
    pub(crate) fn new(
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<Self> {
        if !grant.allows_images() {
            return Err(invalid("import credit cannot create a captured image"));
        }
        let bytes = limits.check(size)? as u64;
        let data = AdmittedBytes::new(grant, owner, bytes)?;
        let mut builder = Self::from_bytes(size, limits, grant, data)?;
        for pixel in builder.pixels_mut().chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(builder)
    }
    /// The only adoption path accepts bytes whose allocation already owns a slice.
    pub(crate) fn from_bytes(
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        bytes: AdmittedBytes,
    ) -> io::Result<Self> {
        if !grant.allows_images() {
            return Err(invalid("import credit cannot create a captured image"));
        }
        if bytes.len() != limits.check(size)? || !bytes.belongs_to(grant) {
            return Err(invalid(
                "admitted capture bytes have wrong geometry or grant",
            ));
        }
        let (data, pixels) = bytes.into_parts();
        let backing = pixels.grant()?;
        let image = BgraImage::from_vec_with_limits(size, data, limits)?;
        Ok(Self {
            image,
            source: grant.source().clone(),
            generation: grant.generation(),
            regions: None,
            pixels,
            grant: backing,
        })
    }
    pub(crate) fn new_local(
        size: FrameSize,
        limits: FrameLimits,
        admission: &Arc<ImageAdmission>,
        role: ImageRole,
        source: SourceKey,
        generation: CaptureGeneration,
    ) -> io::Result<Self> {
        let bytes = limits.check(size)? as u64;
        let data = AdmittedBytes::from_slice(admission.try_claim(role, bytes)?)?;
        let mut builder =
            Self::from_local_bytes(size, limits, admission, source, generation, data)?;
        for pixel in builder.pixels_mut().chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(builder)
    }
    /// Takes an already admitted allocation; no naked Vec adoption is exposed.
    pub(crate) fn from_local_bytes(
        size: FrameSize,
        limits: FrameLimits,
        admission: &Arc<ImageAdmission>,
        source: SourceKey,
        generation: CaptureGeneration,
        bytes: AdmittedBytes,
    ) -> io::Result<Self> {
        if bytes.len() != limits.check(size)? || !bytes.belongs_to_images(admission) {
            return Err(invalid(
                "local image bytes have wrong geometry or admission",
            ));
        }
        let (data, pixels) = bytes.into_parts();
        let grant = CaptureGrant::for_local_image(admission.clone(), source.clone(), generation);
        let image = BgraImage::from_vec_with_limits(size, data, limits)?;
        Ok(Self {
            image,
            source,
            generation,
            regions: None,
            pixels,
            grant,
        })
    }
    pub(crate) fn pixels_mut(&mut self) -> &mut [u8] {
        self.image.data_mut()
    }
    pub(crate) fn size(&self) -> FrameSize {
        self.image.size()
    }
    pub(crate) fn view(&self) -> BgraView<'_> {
        self.image.view()
    }
    pub(crate) fn row_mut(&mut self, y: u32) -> Option<&mut [u8]> {
        (y < self.image.size().height).then(|| self.image.row_mut(y))
    }
    pub(crate) fn copy_from(&mut self, source: &BgraView<'_>) -> io::Result<()> {
        if source.size() != self.image.size() {
            return Err(invalid("capture copy geometry mismatch"));
        }
        for y in 0..source.size().height {
            self.image.row_mut(y).copy_from_slice(source.row(y));
        }
        Ok(())
    }
    pub(crate) fn set_regions(&mut self, regions: AdmittedRegions) -> io::Result<()> {
        if regions.size() != self.image.size() || !regions.same_grant(&self.pixels) {
            return Err(invalid("capture regions have wrong geometry or grant"));
        }
        let (data, slice) = regions.into_parts();
        self.image.set_regions(data)?;
        self.regions = Some(slice);
        Ok(())
    }
    pub(crate) fn freeze(self) -> CapturedImage {
        CapturedImage {
            storage: Arc::new(PixelStorage {
                image: self.image,
                source: self.source,
                generation: self.generation,
                _regions: self.regions,
                _pixels: self.pixels,
                grant: self.grant,
            }),
        }
    }
}
impl CapturedImage {
    pub(crate) fn retain_lifetime(
        &self,
        lifetime: Arc<dyn super::capture_grant::CaptureCreditLifetime>,
    ) -> io::Result<()> {
        self.storage.grant.retain_lifetime(lifetime)
    }
    pub(crate) fn image(&self) -> &BgraImage {
        &self.storage.image
    }
    pub(crate) fn size(&self) -> FrameSize {
        self.storage.image.size()
    }
    pub(crate) fn view(&self) -> BgraView<'_> {
        self.storage.image.view()
    }
    pub(crate) fn source(&self) -> &SourceKey {
        &self.storage.source
    }
    pub(crate) fn generation(&self) -> CaptureGeneration {
        self.storage.generation
    }
    pub(crate) fn grant(&self) -> CaptureGrant {
        self.storage.grant.clone()
    }
    pub(crate) fn image_admission(&self) -> Option<Arc<ImageAdmission>> {
        self.storage.grant.image_admission()
    }
    pub(crate) fn image_role(&self) -> Option<ImageRole> {
        self.storage._pixels.image_role()
    }
    pub(crate) fn shares_pixels(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
    pub(crate) fn downscale(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<Self> {
        grant.check_identity(self.source(), self.generation())?;
        limits.check(size)?;
        let from = self.size();
        if size.width > from.width || size.height > from.height {
            return Err(invalid("downscale target is larger than the source"));
        }
        if size == from {
            return Ok(self.clone());
        }
        let mut target = CaptureImageBuilder::new(size, limits, grant, owner)?;
        let span = |index: u32, target: u32, source: u32| {
            let start = u64::from(index) * u64::from(source) / u64::from(target);
            let end = u64::from(index + 1) * u64::from(source) / u64::from(target);
            (start as usize, end.max(start + 1) as usize)
        };
        let view = self.view();
        // Calculate spans on demand; this path needs no unadmitted columns Vec.
        for y in 0..size.height {
            let (top, bottom) = span(y, size.height, from.height);
            for (x, pixel) in target.image.row_mut(y).chunks_exact_mut(4).enumerate() {
                let (left, right) = span(x as u32, size.width, from.width);
                let mut sums = [0u64; 4];
                for line in top..bottom {
                    for source in view.row(line as u32)[left * 4..right * 4].chunks_exact(4) {
                        for (sum, value) in sums.iter_mut().zip(source) {
                            *sum += u64::from(*value);
                        }
                    }
                }
                let count = (bottom - top) as u64 * (right - left) as u64;
                for (value, sum) in pixel.iter_mut().zip(sums) {
                    *value = ((sum + count / 2) / count) as u8;
                }
            }
        }
        Ok(target.freeze())
    }
    pub(crate) fn rotate(
        &self,
        rotation: BgraRotation,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<Self> {
        grant.check_identity(self.source(), self.generation())?;
        limits.check(rotation.size(self.size()))?;
        if rotation == BgraRotation::Identity {
            return Ok(self.clone());
        }
        let from = self.size();
        let mut target = CaptureImageBuilder::new(rotation.size(from), limits, grant, owner)?;
        let view = self.view();
        for y in 0..from.height {
            for (x, pixel) in view.row(y).chunks_exact(4).enumerate() {
                let x = x as u32;
                let (dx, dy) = match rotation {
                    BgraRotation::Clockwise90 => (from.height - 1 - y, x),
                    BgraRotation::Clockwise180 => (from.width - 1 - x, from.height - 1 - y),
                    BgraRotation::Clockwise270 => (y, from.width - 1 - x),
                    BgraRotation::Identity => (x, y),
                };
                target.image.row_mut(dy)[dx as usize * 4..dx as usize * 4 + 4]
                    .copy_from_slice(pixel);
            }
        }
        Ok(target.freeze())
    }
}
