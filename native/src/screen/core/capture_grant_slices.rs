//! Non-cloneable slice ownership and resource destruction ordering.
use std::io;
use std::sync::Arc;

use super::super::capture_plan::CaptureOwner;
use super::super::image_admission::{ImageAdmission, ImageClaim, ImageRole};
use super::super::wire::invalid;
use super::{Backing, CaptureGrant, GrantInner, MemberScope};

pub(crate) struct CaptureSlice {
    pub(super) grant: Option<Arc<GrantInner>>,
    pub(super) member: Option<Arc<MemberScope>>,
    pub(super) owner: CaptureOwner,
    pub(super) bytes: u64,
    pub(super) image: Option<ImageClaim>,
}

/// Holds native resources in the same destruction order as pixel storage.
/// The closure passed to allocate runs only after the slice was acquired.
pub(crate) struct AdmittedResource<T> {
    resource: T,
    _slice: CaptureSlice,
}

impl<T> AdmittedResource<T> {
    pub(crate) fn resource(&self) -> &T {
        &self.resource
    }
}

impl CaptureSlice {
    pub(in super::super) fn from_image_claim(image: ImageClaim, bytes: u64) -> Self {
        // This legacy classification never authorizes a local allocation;
        // image_role is the exact local ownership class.
        let owner = match image.role() {
            ImageRole::EncodedInput => CaptureOwner::CodecInput,
            ImageRole::DecodedInput | ImageRole::ViewerInput => CaptureOwner::SourcePixels,
            ImageRole::ColorSource => CaptureOwner::CompositionPixels,
            ImageRole::ScaledOutput => CaptureOwner::ScaledPixels,
            ImageRole::FilterScratch => CaptureOwner::FilterScratch,
            ImageRole::UploadStaging => CaptureOwner::Readback,
            ImageRole::RenderSurface | ImageRole::AndroidBitmap => CaptureOwner::NativeSurface,
            ImageRole::ImageMetadata => CaptureOwner::Metadata,
        };
        Self {
            grant: None,
            member: None,
            owner,
            bytes,
            image: Some(image),
        }
    }

    pub(crate) fn image_role(&self) -> Option<ImageRole> {
        self.image.as_ref().map(ImageClaim::role)
    }

    pub(crate) fn image_admission(&self) -> Option<Arc<ImageAdmission>> {
        self.image.as_ref().map(ImageClaim::admission)
    }

    pub(crate) fn belongs_to_images(&self, admission: &Arc<ImageAdmission>) -> bool {
        self.image_admission()
            .is_some_and(|own| Arc::ptr_eq(&own, admission))
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(in super::super) fn allows_images(&self) -> bool {
        self.image.is_some()
            || self
                .grant
                .as_ref()
                .is_some_and(|grant| !grant.plan.is_import())
    }

    pub(crate) fn owner(&self) -> CaptureOwner {
        self.owner
    }

    pub(crate) fn grant(&self) -> io::Result<CaptureGrant> {
        if self.image.is_some() {
            return Err(invalid(
                "local image slice uses ImageAdmission instead of CaptureGrant",
            ));
        }
        let inner = self
            .grant
            .as_ref()
            .ok_or_else(|| invalid("capture slice already consumed"))?;
        Ok(CaptureGrant {
            inner: inner.clone(),
            member: self.member.clone(),
        })
    }

    pub(in super::super) fn check_allocation(&self) -> io::Result<()> {
        if let Some(image) = &self.image {
            image.check_allocation(self.bytes)?;
        }
        Ok(())
    }

    pub(crate) fn allocate<T>(
        self,
        create: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<AdmittedResource<T>> {
        self.check_allocation()?;
        let resource = create()?;
        Ok(AdmittedResource {
            resource,
            _slice: self,
        })
    }

    pub(crate) fn belongs_to(&self, grant: &CaptureGrant) -> bool {
        self.grant
            .as_ref()
            .is_some_and(|inner| Arc::ptr_eq(inner, &grant.inner))
            && same_member(&self.member, &grant.member)
    }

    pub(crate) fn same_grant(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (&self.image, &other.image) {
            return a.same_admission(b);
        }
        match (&self.grant, &other.grant) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) && same_member(&self.member, &other.member),
            _ => false,
        }
    }

    pub(crate) fn split(mut self, first_bytes: u64) -> io::Result<(Self, Self)> {
        if first_bytes > self.bytes() {
            return Err(invalid("capture split exceeds slice"));
        }
        let first = Self {
            grant: self.grant.clone(),
            member: self.member.clone(),
            owner: self.owner,
            bytes: first_bytes,
            image: self.image.as_ref().map(ImageClaim::alias),
        };
        let second = Self {
            grant: self.grant.take(),
            member: self.member.take(),
            owner: self.owner,
            bytes: self.bytes - first_bytes,
            image: self.image.take(),
        };
        self.bytes = 0;
        Ok((first, second))
    }

    pub(crate) fn merge(mut self, mut other: Self) -> io::Result<Self> {
        if self.image_role() != other.image_role()
            || matches!((&self.image, &other.image), (Some(a), Some(b)) if !a.same_claim(b))
        {
            return Err(invalid(
                "local image merge requires segments of the same original claim",
            ));
        }
        if self.owner != other.owner || !self.same_grant(&other) {
            return Err(invalid("capture merge has different owner or grant"));
        }
        let bytes = self
            .bytes
            .checked_add(other.bytes)
            .ok_or_else(|| invalid("capture merge overflow"))?;
        let grant = self.grant.take();
        other.grant.take();
        let image = self.image.take();
        other.image.take();
        Ok(Self {
            grant,
            member: self.member.take(),
            owner: self.owner,
            bytes,
            image,
        })
    }
}

impl Drop for CaptureSlice {
    fn drop(&mut self) {
        if let Some(image) = self.image.take() {
            image.release(self.bytes);
            return;
        }
        let Some(grant) = self.grant.take() else {
            return;
        };
        let mut usage = grant
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        usage.total -= self.bytes();
        usage.owners[self.owner as usize] -= self.bytes();
        if let Some(member) = &self.member {
            let mut local = member
                .usage
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            local.total -= self.bytes();
            local.owners[self.owner as usize] -= self.bytes();
        }
        if usage.retired {
            if let Backing::Session(session) = &grant.backing {
                // Poison/error cannot invent released parent credit. Retain
                // the previous promise until a later drop or final release.
                if session.reduce_plan(usage.reserved, usage.total).is_ok() {
                    usage.reserved = usage.total;
                }
            }
        }
        drop(usage);
        grant.notify_retired();
    }
}

fn same_member(a: &Option<Arc<MemberScope>>, b: &Option<Arc<MemberScope>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
