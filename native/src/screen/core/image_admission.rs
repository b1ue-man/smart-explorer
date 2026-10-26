//! Exact local image roles backed by the one process-wide W/X memory pool.
use std::io;
use std::sync::{Arc, Mutex};
use super::capture_grant::CaptureSlice;
use super::frame_memory::{self, CapacityCeiling, MemoryFacts, MemoryLease, MemoryPool};
use super::wire::invalid;

const ROLE_COUNT: usize = 10;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum ImageRole {
    EncodedInput, DecodedInput, ViewerInput, ColorSource, ScaledOutput,
    FilterScratch, UploadStaging, RenderSurface, AndroidBitmap, ImageMetadata,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalImagePolicy {
    pub facts: Option<MemoryFacts>,
    pub ceiling: CapacityCeiling,
}
#[derive(Clone, Copy, Debug, Default)]
struct RoleLimit {bytes_each: u64, count: u64, bytes: u64}
/// Limits authorize ownership, never reserve independent pots of memory.
#[derive(Clone, Debug)]
pub(crate) struct ImageOwnerLimits {roles: [RoleLimit; ROLE_COUNT], total: u64}
impl ImageOwnerLimits {
    pub(crate) fn new(total_bytes: u64) -> io::Result<Self> {
        if total_bytes == 0 {return Err(invalid("empty image ownership limit"));}
        Ok(Self {roles: [RoleLimit::default(); ROLE_COUNT], total: total_bytes})
    }
    pub(crate) fn with_role(mut self, role: ImageRole, bytes_each: u64, count: u64) -> io::Result<Self> {
        if bytes_each == 0 || count == 0 || bytes_each > self.total {
            return Err(invalid("invalid image role ownership limit"));
        }
        let bytes = bytes_each.checked_mul(count).ok_or_else(|| invalid("image role capacity overflow"))?;
        self.roles[role as usize] = RoleLimit {bytes_each, count, bytes};
        Ok(self)
    }
    pub(crate) fn total_bytes(&self) -> u64 {self.total}
}
#[derive(Default)]
struct Usage {bytes: [u64; ROLE_COUNT], counts: [u64; ROLE_COUNT], total: u64}
pub(crate) struct ImageAdmission {
    pool: Arc<MemoryPool>,
    policy: LocalImagePolicy,
    limits: ImageOwnerLimits,
    usage: Mutex<Usage>,
}
/// A split keeps one logical role slot until its final segment is gone.
/// Individual segments release their own bytes, so dropping a prefix does not
/// pretend the remaining allocation has ended and does not retain its bytes.
pub(super) struct ImageClaim {inner: Arc<ClaimInner>}
struct ClaimInner {
    admission: Arc<ImageAdmission>,
    role: ImageRole,
    root: Mutex<MemoryLease>,
}
impl ImageAdmission {
    pub(super) fn in_pool(pool: Arc<MemoryPool>, policy: LocalImagePolicy, limits: ImageOwnerLimits) -> Arc<Self> {
        Arc::new(Self {pool, policy, limits, usage: Mutex::new(Usage::default())})
    }
    #[cfg(test)]
    pub(crate) fn in_test_pool(pool: Arc<MemoryPool>, policy: LocalImagePolicy, limits: ImageOwnerLimits) -> Arc<Self> {
        Self::in_pool(pool, policy, limits)
    }
    pub(crate) fn try_claim(self: &Arc<Self>, role: ImageRole, bytes: u64) -> io::Result<CaptureSlice> {
        let limit = self.limits.roles[role as usize];
        if bytes == 0 || bytes > limit.bytes_each || bytes > self.limits.total {
            return Err(invalid("single image claim exceeds its role or total limit"));
        }
        // Every claim/drop takes role usage before the root pool. There is no
        // root-to-role lock path. Failure leaves both ledgers untouched.
        let mut usage = self.usage.lock().map_err(|_| io::Error::other("image ownership accounting poisoned"))?;
        let count = usage.counts[role as usize].checked_add(1).ok_or_else(|| invalid("image role count overflow"))?;
        let role_bytes = usage.bytes[role as usize].checked_add(bytes).ok_or_else(|| invalid("image role byte overflow"))?;
        let total = usage.total.checked_add(bytes).ok_or_else(|| invalid("image total byte overflow"))?;
        if count > limit.count || role_bytes > limit.bytes || total > self.limits.total {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "image ownership slots or bytes are occupied"));
        }
        let root = self.pool.reserve_exact(self.policy.facts, self.policy.ceiling, bytes)?;
        usage.counts[role as usize] = count;
        usage.bytes[role as usize] = role_bytes;
        usage.total = total;
        let claim = ImageClaim {inner: Arc::new(ClaimInner {admission: self.clone(), role, root: Mutex::new(root)})};
        Ok(CaptureSlice::from_image_claim(claim, bytes))
    }
    pub(crate) fn live_bytes(&self) -> io::Result<u64> {
        Ok(self.usage.lock().map_err(|_| io::Error::other("image ownership accounting poisoned"))?.total)
    }
    pub(crate) fn live_count(&self, role: ImageRole) -> io::Result<u64> {
        Ok(self.usage.lock().map_err(|_| io::Error::other("image ownership accounting poisoned"))?.counts[role as usize])
    }
}
impl ImageClaim {
    pub(super) fn alias(&self) -> Self {Self {inner: self.inner.clone()}}
    pub(super) fn role(&self) -> ImageRole {self.inner.role}
    pub(super) fn admission(&self) -> Arc<ImageAdmission> {self.inner.admission.clone()}
    pub(super) fn same_claim(&self, other: &Self) -> bool {Arc::ptr_eq(&self.inner, &other.inner)}
    pub(super) fn same_admission(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner.admission, &other.inner.admission)
    }
    pub(super) fn check_allocation(&self, bytes: u64) -> io::Result<()> {
        let root = self.inner.root.lock().map_err(|_| io::Error::other("image claim accounting poisoned"))?;
        if root.bytes() != bytes {
            return Err(invalid("split image claim cannot authorize an additional resource"));
        }
        Ok(())
    }
    pub(super) fn release(&self, bytes: u64) {
        let mut usage = self.inner.admission.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut root = self.inner.root.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let remaining = root.bytes() - bytes;
        root.release_to(remaining);
        usage.bytes[self.inner.role as usize] -= bytes;
        usage.total -= bytes;
    }
}
impl Drop for ClaimInner {
    fn drop(&mut self) {
        let mut usage = self.admission.usage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        usage.counts[self.role as usize] -= 1;
    }
}
pub(crate) fn admit_images(policy: LocalImagePolicy, limits: ImageOwnerLimits) -> io::Result<Arc<ImageAdmission>> {
    Ok(ImageAdmission::in_pool(frame_memory::pool().clone(), policy, limits))
}
/// A lifetime anchor grants no right to allocate a second image. Callers need
/// the contract allocator or the explicit local role before creating a copy.
pub(crate) enum ImageMemory {Contract(Arc<MemoryLease>), Slice(CaptureSlice)}
pub(crate) type ImageHold = Arc<ImageMemory>;
impl ImageMemory {
    pub(crate) fn from_slice(slice: CaptureSlice) -> ImageHold {Arc::new(Self::Slice(slice))}
    pub(crate) fn from_contract(lease: Arc<MemoryLease>) -> ImageHold {Arc::new(Self::Contract(lease))}
    pub(crate) fn bytes(&self) -> u64 {
        match self {Self::Contract(lease) => lease.bytes(), Self::Slice(slice) => slice.bytes()}
    }
}
