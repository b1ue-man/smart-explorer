//! Versioned end-to-end size evidence. A missing offer keeps the v1 limits.
use std::io;
use serde::{Deserialize, Serialize};
use super::pixel_convert::FrameSize;
use super::wire::invalid;

pub(crate) const FRAME_PROTOCOL: u8 = 2;
/// Wire coordinates are u16; the implementation admits only measured texture offers.
pub(crate) const MAX_RESOURCE_SIDE: u32 = 16_384;
/// Hard allocation ceiling, not a claim that any device supports these dimensions.
pub(crate) const MAX_WORKING_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const WORKING_IMAGES: u64 = 8;
/// V2 also reserves CPU resampling and old/new renderer output ownership.
pub(crate) const VERSIONED_WORKING_IMAGES:u64=24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FrameLimits {
    pub max_size: FrameSize,
    pub max_pixels: u64,
    pub working_bytes: u64,
}
impl FrameLimits {
    /// Allocation ceiling shared by versioned native paths; device facts narrow it.
    pub(crate) const RESOURCE:Self=Self {max_size:FrameSize::new(MAX_RESOURCE_SIDE,MAX_RESOURCE_SIDE),
        max_pixels:MAX_WORKING_BYTES/WORKING_IMAGES/4,working_bytes:MAX_WORKING_BYTES};
    pub(crate) const LEGACY: Self = Self {
        max_size: FrameSize::new(4096, 4096),
        max_pixels: super::pixel_convert::MAX_FRAME_PIXELS,
        working_bytes: MAX_WORKING_BYTES,
    };
    pub(crate) fn validate(self) -> io::Result<()> {
        if self.max_size.width == 0 || self.max_size.height == 0
            || self.max_size.width > MAX_RESOURCE_SIDE || self.max_size.height > MAX_RESOURCE_SIDE
            || self.max_pixels == 0 || self.working_bytes < WORKING_IMAGES * 4 || self.working_bytes > MAX_WORKING_BYTES {
            return Err(invalid("invalid screen resource capability"));
        }
        Ok(())
    }
    pub(crate) fn check(self, size: FrameSize) -> io::Result<usize> {
        self.validate()?;
        let bytes = u64::from(size.width).checked_mul(u64::from(size.height))
            .and_then(|n| n.checked_mul(4)).ok_or_else(|| invalid("screen byte size overflow"))?;
        if size.width == 0 || size.height == 0 || size.width > self.max_size.width
            || size.height > self.max_size.height || size.pixels() > self.max_pixels
            || bytes.checked_mul(WORKING_IMAGES).is_none_or(|n| n > self.working_bytes) {
            return Err(invalid("screen frame exceeds negotiated dimensions or working memory"));
        }
        usize::try_from(bytes).map_err(|_| invalid("screen byte size exceeds address space"))
    }
    /// An observed allocator budget can only narrow the existing offer.
    pub(crate) fn with_working_budget(mut self,bytes:u64)->io::Result<Self> {
        self.working_bytes=self.working_bytes.min(bytes);
        self.max_pixels=self.max_pixels.min(self.working_bytes/VERSIONED_WORKING_IMAGES/4);
        self.validate()?;
        while super::frame_size_caps::PacketBudget::for_contract(Some(FrameContract {
            version:FRAME_PROTOCOL,limits:self,hybrid_regions:false,
        })).is_err() {
            self.max_pixels/=2;
            if self.max_pixels==0 {return Err(invalid("working memory cannot hold a screen frame transaction"));}
        }
        Ok(self)
    }
    pub(crate) fn intersect(self, rhs: Self) -> Self {
        Self { max_size: FrameSize::new(self.max_size.width.min(rhs.max_size.width),
            self.max_size.height.min(rhs.max_size.height)), max_pixels: self.max_pixels.min(rhs.max_pixels),
            working_bytes: self.working_bytes.min(rhs.working_bytes) }
    }
    pub(crate) fn fit(self, source: FrameSize) -> FrameSize {
        let mut size = source.fit_within(self.max_size);
        // Preserve aspect ratio while satisfying the actual pixel/memory budget.
        let pixels = self.max_pixels.min(self.working_bytes / WORKING_IMAGES / 4);
        if size.pixels() > pixels {
            let scale = (pixels as f64 / size.pixels() as f64).sqrt();
            size = FrameSize::new((size.width as f64 * scale).floor().max(1.0) as u32,
                (size.height as f64 * scale).floor().max(1.0) as u32);
        }
        size
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FrameCapabilities {
    pub version: u8,
    /// Capture and encoder on hosts, decoder on viewers. CodecOffer still narrows it.
    pub frames: FrameLimits,
    /// Actual renderer texture limit, not the display's desktop dimensions.
    pub texture_size: FrameSize,
    pub hybrid_regions: bool,
}
impl FrameCapabilities {
    pub(crate) fn resources(texture_size:FrameSize)->Self {
        let texture_size=FrameSize::new(texture_size.width.min(MAX_RESOURCE_SIDE),texture_size.height.min(MAX_RESOURCE_SIDE));
        Self {version:FRAME_PROTOCOL,frames:FrameLimits {max_size:texture_size,
            max_pixels:MAX_WORKING_BYTES/VERSIONED_WORKING_IMAGES/4,..FrameLimits::RESOURCE},texture_size,hybrid_regions:true}
    }
    pub(crate) fn with_working_budget(mut self,bytes:u64)->io::Result<Self> {
        self.frames=self.frames.with_working_budget(bytes)?;self.validate()?;Ok(self)
    }
    pub(crate) fn legacy_resources(texture_size: FrameSize) -> Self {
        Self { version: FRAME_PROTOCOL, frames: FrameLimits {
            max_pixels:FrameLimits::LEGACY.max_pixels.min(MAX_WORKING_BYTES/VERSIONED_WORKING_IMAGES/4),
            ..FrameLimits::LEGACY}, texture_size, hybrid_regions: true }
    }
    pub(crate) fn validate(self) -> io::Result<()> {
        self.frames.validate()?;
        if self.version != FRAME_PROTOCOL || self.texture_size.width == 0 || self.texture_size.height == 0
            || self.texture_size.width > MAX_RESOURCE_SIDE || self.texture_size.height > MAX_RESOURCE_SIDE {
            return Err(invalid("unsupported screen frame capability"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FrameContract {
    pub version: u8,
    pub limits: FrameLimits,
    pub hybrid_regions: bool,
}
impl FrameContract {
    pub(crate) fn negotiate(host: Option<FrameCapabilities>, viewer: Option<FrameCapabilities>) -> io::Result<Option<Self>> {
        let (Some(host), Some(viewer)) = (host, viewer) else { return Ok(None); };
        host.validate()?; viewer.validate()?;
        let mut limits = host.frames.intersect(viewer.frames);
        limits.max_pixels=limits.max_pixels.min(limits.working_bytes/VERSIONED_WORKING_IMAGES/4);
        limits.max_size = FrameSize::new(limits.max_size.width.min(host.texture_size.width).min(viewer.texture_size.width),
            limits.max_size.height.min(host.texture_size.height).min(viewer.texture_size.height));
        let contract=Self { version: FRAME_PROTOCOL, limits, hybrid_regions: host.hybrid_regions && viewer.hybrid_regions };
        contract.validate()?;
        Ok(Some(contract))
    }
    pub(crate) fn validate(self) -> io::Result<()> {
        if self.version != FRAME_PROTOCOL { return Err(invalid("unsupported screen frame contract")); }
        self.limits.validate()?;
        if self.limits.max_pixels>self.limits.working_bytes/VERSIONED_WORKING_IMAGES/4 {
            return Err(invalid("frame contract overstates its pixel working budget"));
        }
        super::frame_size_caps::PacketBudget::for_contract(Some(self))?;
        Ok(())
    }
}
