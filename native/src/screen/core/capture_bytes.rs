//! Fixed-capacity byte and rectangle storage, admitted before allocation.
use std::io;
use std::sync::Arc;
use super::image_admission::ImageAdmission;
use super::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use super::frame_regions::{FrameRegions, MoveRect};
use super::pixel_convert::{FrameSize, Rect};
use super::wire::invalid;

pub(crate) struct AdmittedBytes {
    data: Vec<u8>,
    capacity: usize,
    // Declaration order destroys the byte allocation before its lease.
    slice: CaptureSlice,
}
impl AdmittedBytes {
    pub(crate) fn new(grant: &CaptureGrant, owner: CaptureOwner, bytes: u64) -> io::Result<Self> {
        let mut storage = Self::buffer(grant, owner, bytes)?;
        storage.data.resize(storage.capacity, 0);
        Ok(storage)
    }
    pub(crate) fn buffer(grant: &CaptureGrant, owner: CaptureOwner, capacity: u64) -> io::Result<Self> {
        if !grant.allows_images() {return Err(invalid("import credit cannot authorize image bytes"));}
        let size = usize::try_from(capacity).map_err(|_| invalid("capture bytes exceed address space"))?;
        let slice = grant.claim(owner, capacity)?;
        let mut data = Vec::new();
        data.try_reserve_exact(size).map_err(|_| io::Error::other("admitted byte allocation failed"))?;
        Ok(Self {data, capacity: size, slice})
    }
    /// Consumes the exact local/capture promise before allocating any bytes.
    pub(crate) fn buffer_from_slice(slice: CaptureSlice) -> io::Result<Self> {
        if !slice.allows_images() {return Err(invalid("import slice cannot authorize image bytes"));}
        slice.check_allocation()?;
        let capacity = usize::try_from(slice.bytes()).map_err(|_| invalid("admitted bytes exceed address space"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(capacity).map_err(|_| io::Error::other("admitted byte allocation failed"))?;
        Ok(Self {data, capacity, slice})
    }
    pub(crate) fn from_slice(slice: CaptureSlice) -> io::Result<Self> {
        let mut storage = Self::buffer_from_slice(slice)?;
        storage.data.resize(storage.capacity, 0);
        Ok(storage)
    }
    pub(crate) fn image_admission(&self) -> Option<Arc<ImageAdmission>> {self.slice.image_admission()}
    pub(crate) fn freeze(self) -> FrozenBytes {FrozenBytes {storage: Arc::new(self)}}
    pub(super) fn belongs_to_images(&self, admission: &Arc<ImageAdmission>) -> bool {
        self.slice.belongs_to_images(admission)
    }
    pub(crate) fn copy(grant: &CaptureGrant, owner: CaptureOwner, bytes: &[u8]) -> io::Result<Self> {
        let mut storage = Self::buffer(grant, owner, bytes.len() as u64)?;
        storage.extend_from_slice(bytes)?;
        Ok(storage)
    }
    /// Runs a bounded library allocator only after its promise exists. The
    /// caller must establish the library's allocation bound before this call;
    /// the returned capacity check cannot retroactively authorize allocation.
    pub(crate) fn from_allocator(
        grant: &CaptureGrant,
        owner: CaptureOwner,
        bytes: u64,
        allocate: impl FnOnce() -> io::Result<Vec<u8>>,
    ) -> io::Result<Self> {
        if !grant.allows_images() {return Err(invalid("import credit cannot authorize image bytes"));}
        let maximum = usize::try_from(bytes)
            .map_err(|_| invalid("capture bytes exceed address space"))?;
        let slice = grant.claim(owner, bytes)?;
        let data = allocate()?;
        if data.capacity() > maximum || data.len() > maximum {
            return Err(invalid("library allocation exceeds admitted capacity"));
        }
        // No later resize may trigger geometric Vec growth beyond this exact
        // allocation, even if the promised maximum was larger than its result.
        let capacity = data.capacity();
        Ok(Self { data, capacity, slice })
    }
    pub(crate) fn as_slice(&self) -> &[u8] {&self.data}
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {&mut self.data}
    pub(crate) fn len(&self) -> usize {self.data.len()}
    pub(crate) fn is_empty(&self) -> bool {self.data.is_empty()}
    pub(crate) fn capacity(&self) -> u64 {self.capacity as u64}
    pub(crate) fn grant(&self) -> io::Result<CaptureGrant> {self.slice.grant()}
    pub(super) fn from_admitted_parts(data: Vec<u8>, slice: CaptureSlice) -> io::Result<Self> {
        if !slice.allows_images() {
            drop(data);
            return Err(invalid("import slice cannot own image bytes"));
        }
        if data.capacity() as u64 > slice.bytes() {
            drop(data);
            return Err(invalid("admitted byte parts exceed their slice"));
        }
        let capacity = data.capacity();
        Ok(Self {data, capacity, slice})
    }
    pub(crate) fn resize(&mut self, length: usize) -> io::Result<()> {
        if length > self.capacity {return Err(invalid("admitted bytes cannot grow their allocation"));}
        self.data.resize(length, 0);
        Ok(())
    }
    pub(crate) fn extend_from_slice(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.data.len().checked_add(bytes.len()).is_none_or(|n| n > self.capacity) {
            return Err(invalid("admitted bytes exceed fixed capacity"));
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    pub(super) fn belongs_to(&self, grant: &CaptureGrant) -> bool {self.slice.belongs_to(grant)}
    pub(super) fn into_parts(self) -> (Vec<u8>, CaptureSlice) {(self.data, self.slice)}
}

/// Immutable aliases retain the same allocation and exact original slice.
#[derive(Clone)]
pub(crate) struct FrozenBytes {storage: Arc<AdmittedBytes>}
impl FrozenBytes {
    pub(crate) fn as_slice(&self) -> &[u8] {self.storage.as_slice()}
    pub(crate) fn len(&self) -> usize {self.storage.len()}
    pub(crate) fn is_empty(&self) -> bool {self.storage.is_empty()}
    pub(crate) fn capacity(&self) -> u64 {self.storage.capacity()}
    pub(crate) fn grant(&self) -> io::Result<CaptureGrant> {self.storage.grant()}
    pub(crate) fn image_admission(&self) -> Option<Arc<ImageAdmission>> {self.storage.image_admission()}
    pub(crate) fn shares_bytes(&self, other: &Self) -> bool {Arc::ptr_eq(&self.storage, &other.storage)}
}

pub(crate) struct AdmittedRegions {
    regions: FrameRegions,
    size: FrameSize,
    move_capacity: usize,
    dirty_capacity: usize,
    // The vectors are destroyed before the Metadata slice.
    slice: CaptureSlice,
}
impl AdmittedRegions {
    pub(crate) fn new(size: FrameSize, grant: &CaptureGrant,
        move_capacity: u64, dirty_capacity: u64) -> io::Result<Self> {
        if size.width == 0 || size.height == 0 {return Err(invalid("capture regions have unknown geometry"));}
        let entries = move_capacity.checked_add(dirty_capacity).ok_or_else(|| invalid("capture region count overflow"))?;
        let maximum = u64::from(size.width.div_ceil(64)).checked_mul(u64::from(size.height.div_ceil(64)))
            .and_then(|n| n.checked_mul(2)).ok_or_else(|| invalid("capture region grid overflow"))?;
        if entries > maximum {return Err(invalid("too many admitted capture regions"));}
        let bytes = Self::required_bytes(move_capacity, dirty_capacity)?;
        let moves = usize::try_from(move_capacity).map_err(|_| invalid("capture move capacity exceeds address space"))?;
        let dirty = usize::try_from(dirty_capacity).map_err(|_| invalid("capture dirty capacity exceeds address space"))?;
        let slice = grant.claim(CaptureOwner::Metadata, bytes)?;
        let mut regions = FrameRegions::default();
        regions.moves.try_reserve_exact(moves).map_err(|_| io::Error::other("admitted move allocation failed"))?;
        regions.dirty.try_reserve_exact(dirty).map_err(|_| io::Error::other("admitted dirty allocation failed"))?;
        Ok(Self {regions, size, move_capacity: moves, dirty_capacity: dirty, slice})
    }
    pub(crate) fn required_bytes(moves: u64, dirty: u64) -> io::Result<u64> {
        moves.checked_mul(std::mem::size_of::<MoveRect>() as u64)
            .and_then(|n| dirty.checked_mul(std::mem::size_of::<Rect>() as u64).and_then(|d| n.checked_add(d)))
            .ok_or_else(|| invalid("capture region byte size overflow"))
    }
    pub(crate) fn push_move(&mut self, moved: MoveRect) -> io::Result<()> {
        moved.validate(self.size)?;
        if self.regions.moves.len() == self.move_capacity {return Err(invalid("admitted move capacity exhausted"));}
        self.regions.moves.push(moved);
        Ok(())
    }
    pub(crate) fn push_dirty(&mut self, dirty: Rect) -> io::Result<()> {
        if !dirty.fits(self.size) {return Err(invalid("admitted dirty rectangle exceeds frame"));}
        if self.regions.dirty.len() == self.dirty_capacity {return Err(invalid("admitted dirty capacity exhausted"));}
        self.regions.dirty.push(dirty);
        Ok(())
    }
    pub(crate) fn regions(&self) -> &FrameRegions {&self.regions}
    pub(super) fn size(&self) -> FrameSize {self.size}
    pub(super) fn same_grant(&self, pixels: &CaptureSlice) -> bool {self.slice.same_grant(pixels)}
    pub(super) fn into_parts(self) -> (FrameRegions, CaptureSlice) {(self.regions, self.slice)}
}
