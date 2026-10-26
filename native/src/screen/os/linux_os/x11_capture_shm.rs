//! SHM mapping owns its prior promise until checked server detach or connection close.
use super::*;
use crate::screen::core::capture_admission::CaptureSlice;
/// A memfd shared with the X server.
pub(super) struct Segment {
    pub(super) id: u32,
    map: *mut c_void,
    pub(super) len: usize,
    slice: Option<CaptureSlice>,
}

// SAFETY: the mapping is only read through `&self`, by the owner.
unsafe impl Send for Segment {}

impl Segment {
    pub(super) fn create(conn: &CaptureConnection, len: usize, grant: &CaptureGrant) -> io::Result<Self> {
        let slice = grant.claim(CaptureOwner::NativeSurface, len as u64)?;
        // SAFETY: plain system calls; the descriptor is owned right away.
        let (fd, map) = unsafe {
            let raw = libc::memfd_create(c"se-screen".as_ptr(), libc::MFD_CLOEXEC);
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(raw);
            if libc::ftruncate(raw, len as libc::off_t) != 0 {
                return Err(io::Error::last_os_error());
            }
            let map = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                raw,
                0,
            );
            if map == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            (fd, map)
        };
        // From here on the mapping goes away with the segment.
        let mut segment = Self { id: 0, map, len, slice: Some(slice) };
        segment.id = conn.generate_id().map_err(failed)?;
        // The server gets its own descriptor; ours closes with `fd`.
        let shared = fd.try_clone()?;
        let attach = conn.shm_attach_fd(segment.id, shared, false).map_err(failed)
            .and_then(|cookie| cookie.check().map_err(failed));
        if let Err(error) = attach {
            if let Some(slice) = segment.slice.take() {conn.stream().gate().retain(slice);}
            return Err(error);
        }
        Ok(segment)
    }

    pub(super) fn bytes(&self) -> &[u8] {
        // SAFETY: `map` maps `len` readable bytes until `release`.
        unsafe { std::slice::from_raw_parts(self.map.cast::<u8>(), self.len) }
    }

    /// Tells the server to let go of the segment, then unmaps it.
    pub(super) fn release(mut self, conn: &CaptureConnection) {
        if self.id != 0 {
            let detached = conn.shm_detach(self.id).map_err(failed)
                .and_then(|cookie| cookie.check().map_err(failed));
            if detached.is_err() {
                if let Some(slice) = self.slice.take() {conn.stream().gate().retain(slice);}
            }
        }
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        // SAFETY: the mapping is ours and nothing refers to it any more.
        unsafe { libc::munmap(self.map, self.len) };
    }
}

