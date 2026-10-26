//! IMFMediaBuffer identity that keeps its allocation claim until final Release.
//! Never forwards QueryInterface: exposing the inner buffer would let an MFT
//! keep its bytes after releasing this identity and its slice.
use super::mf_common::platform;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use std::io;
use windows::core::{implement, Interface};
use windows::Win32::Media::MediaFoundation::{IMFMediaBuffer, IMFMediaBuffer_Impl};

#[implement(IMFMediaBuffer)]
struct LeasedMediaBuffer {
    // COM releases the native allocation before the corresponding promise.
    inner: IMFMediaBuffer,
    _slice: CaptureSlice,
}

impl IMFMediaBuffer_Impl for LeasedMediaBuffer_Impl {
    fn Lock(
        &self,
        data: *mut *mut u8,
        maximum: *mut u32,
        length: *mut u32,
    ) -> windows::core::Result<()> {
        // SAFETY: this interface has exactly IMFMediaBuffer's contract; the
        // inner reference stays owned by this wrapper throughout the lock.
        unsafe {
            self.inner.Lock(
                data,
                (!maximum.is_null()).then_some(maximum),
                (!length.is_null()).then_some(length),
            )
        }
    }
    fn Unlock(&self) -> windows::core::Result<()> {
        unsafe { self.inner.Unlock() }
    }
    fn GetCurrentLength(&self) -> windows::core::Result<u32> {
        unsafe { self.inner.GetCurrentLength() }
    }
    fn SetCurrentLength(&self, length: u32) -> windows::core::Result<()> {
        unsafe { self.inner.SetCurrentLength(length) }
    }
    fn GetMaxLength(&self) -> windows::core::Result<u32> {
        unsafe { self.inner.GetMaxLength() }
    }
}

/// MFCreateMemoryBuffer allocates exactly its documented cbMaxLength. The
/// claim is acquired before that call, and every returned COM alias owns it.
pub(super) fn create(
    grant: &CaptureGrant,
    owner: CaptureOwner,
    length: usize,
) -> io::Result<IMFMediaBuffer> {
    if length > 64 * 1024 * 1024 {
        return Err(io::Error::other("MF sample exceeds its resource limit"));
    }
    let length =
        u32::try_from(length.max(1)).map_err(|_| io::Error::other("MF buffer size overflow"))?;
    let slice = grant.claim(owner, u64::from(length))?;
    let platform = platform()?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: MF writes one owned IMFMediaBuffer reference.
    unsafe { (platform.create_memory_buffer)(length, &mut raw) }.ok()?;
    let inner = unsafe { IMFMediaBuffer::from_raw(raw) };
    Ok(LeasedMediaBuffer {
        inner,
        _slice: slice,
    }
    .into())
}
