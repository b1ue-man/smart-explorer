//! Global-memory ownership stays local until successful clipboard publication.
use std::{marker::PhantomData, ptr::NonNull, rc::Rc};
use windows::{core::{Error, Result}, Win32::{
    Foundation::{GlobalFree, HANDLE, HGLOBAL},
    System::{DataExchange::SetClipboardData, Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT}},
}};

pub(super) struct OwnedGlobal { handle: HGLOBAL }

impl OwnedGlobal {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > super::MAX_CLIPBOARD_BYTES {
            return Err(super::invalid("Clipboard allocation exceeds its size limit"));
        }
        let owned = Self { handle: unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len())? } };
        {
            let locked = unsafe { LockedGlobal::new(owned.handle)? };
            // This allocation is not published and has no other borrowers.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked.pointer.as_ptr(), bytes.len()); }
        }
        Ok(owned)
    }

    pub(super) fn handle(&self) -> HGLOBAL { self.handle }

    pub(super) fn publish(mut self, format: u32) -> Result<()> {
        unsafe { SetClipboardData(format, HANDLE(self.handle.0))?; }
        // Only successful SetClipboardData transfers ownership to Windows.
        self.handle = HGLOBAL::default();
        Ok(())
    }
}

impl Drop for OwnedGlobal {
    fn drop(&mut self) {
        if !self.handle.0.is_null() {
            // 0.58 projects GlobalFree's successful NULL return as Err. Cleanup
            // must invoke it once, not retry based on that projected Result.
            let _ = unsafe { GlobalFree(self.handle) };
        }
    }
}

pub(super) struct LockedGlobal {
    handle: HGLOBAL,
    pointer: NonNull<u8>,
    length: usize,
    _thread: PhantomData<Rc<()>>,
}

impl LockedGlobal {
    /// The handle must remain owned and valid until this guard is dropped.
    /// Clipboard callers must hold their open clipboard session throughout.
    pub(super) unsafe fn new(handle: HGLOBAL) -> Result<Self> {
        let length = unsafe { GlobalSize(handle) };
        if length == 0 { return Err(super::invalid("Clipboard global memory is empty or invalid")); }
        if length > super::MAX_CLIPBOARD_BYTES {
            return Err(super::invalid("Clipboard data exceeds 64 MiB"));
        }
        let pointer = NonNull::new(unsafe { GlobalLock(handle) }.cast::<u8>())
            .ok_or_else(Error::from_win32)?;
        Ok(Self { handle, pointer, length, _thread: PhantomData })
    }

    pub(super) fn bytes(&self) -> &[u8] {
        // GlobalSize bounds the allocation; the guard holds a successful lock.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.length) }
    }
}

impl Drop for LockedGlobal {
    fn drop(&mut self) {
        // A final successful GlobalUnlock returns FALSE with NO_ERROR; the
        // generated BOOL-to-Result wrapper must not drive retry logic here.
        let _ = unsafe { GlobalUnlock(self.handle) };
    }
}
