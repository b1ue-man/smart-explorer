//! A picture NVENC did not complete in time (review 2026-11-19, Gemini
//! round 3 finding 14, Codex round 5 finding 6). `nvEncodeAPI.h` 12.1: the
//! client unmaps an input only "after ::NvEncLockBitstream() API returns
//! successfully for encode work submitted using the mapped input buffer"
//! (`NvEncUnmapInputResource`), and before `NvEncDestroyEncoder` it "must
//! flush the encoder ... pass a NULL encode picture packet and ... wait for
//! the flush event to be signaled by the encoder in asynchronous mode".
//!
//! A late picture keeps its input mapped (and its capture ring slot, which
//! `GpuInput` gives back after the session went: `VendorEncoder` drops the
//! session first). When the session goes, in this order and each step
//! confirmed before the next: the late picture's own completion (the
//! session's event, `LATE_WAIT_MS`), its bitstream locked and unlocked, the
//! end-of-stream packet sent and its completion awaited on the same event
//! (signalled once per completion, auto-reset; the picture's signal was
//! taken by the first wait), then the input unmapped. A step that is not
//! confirmed leaves the session as it is: its input, output, registrations,
//! event and session are kept (leaked) for the rest of the process rather
//! than freed while the driver may still use them.
use super::*;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

/// `NV_ENC_PIC_FLAG_EOS` of `nvEncodeAPI.h`: the end of the input stream.
const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;
/// Longest wait for each of the two completions.
const LATE_WAIT_MS: u32 = 2_000;

impl NvencSession {
    /// The mapped input of a picture that timed out stays mapped.
    pub(super) fn keep_late(&mut self, mapped: Ptr) {
        self.late = Some(mapped);
    }

    /// Before the session goes: the late picture finished as the API asks,
    /// else everything of the session is kept (see the module comment).
    /// Whether its input may go back to the capture: `false` when it is kept
    /// (Codex round 6, finding 3: `VendorEncoder` then keeps the ring slot).
    pub(super) fn drain_late(&mut self) -> bool {
        let Some(mapped) = self.late.take() else {
            return true;
        };
        if self.late_finished() {
            if let Some(unmap) = self.core.api.functions.unmap_input_resource {
                // SAFETY: mapped by this session; its work completed and its
                // bitstream was locked.
                unsafe { unmap(self.core.encoder, mapped) };
            }
            return true;
        }
        eprintln!("screen encoder nvenc: a late picture did not finish; the session is kept");
        std::mem::forget(std::mem::take(&mut self.registered));
        self.core.output = std::ptr::null_mut();
        self.core.event = std::ptr::null_mut();
        self.core.event_registered = false;
        self.core.encoder = std::ptr::null_mut();
        false
    }

    /// The late picture completed, its bitstream locked, the flush done.
    fn late_finished(&mut self) -> bool {
        let event = self.core.event;
        // SAFETY: a wait on the session's live completion event.
        let waited = |ms| unsafe { WaitForSingleObject(event, ms) } == WAIT_OBJECT_0;
        if !waited(LATE_WAIT_MS) {
            return false;
        }
        let table = self.core.api.table();
        let (Some(lock), Some(unlock), Some(encode)) = (
            table.lock_bitstream,
            table.unlock_bitstream,
            table.encode_picture,
        ) else {
            return false;
        };
        let mut locked = Box::new(NvEncLockBitstream::zeroed());
        locked.version = NV_ENC_LOCK_BITSTREAM_VER;
        locked.flags = LOCK_DO_NOT_WAIT;
        locked.output_bitstream = self.core.output;
        // SAFETY: the completion of this output was signalled; no wait.
        if unsafe { lock(self.core.encoder, &mut *locked) } != NV_ENC_SUCCESS {
            return false;
        }
        // SAFETY: locked above; the late picture's bytes are not used.
        if unsafe { unlock(self.core.encoder, self.core.output) } != NV_ENC_SUCCESS {
            return false;
        }
        let mut params = Box::new(NvEncPicParams::zeroed());
        params.version = NV_ENC_PIC_PARAMS_VER;
        params.encode_pic_flags = NV_ENC_PIC_FLAG_EOS;
        params.completion_event = event;
        // SAFETY: valid async session; the end-of-stream packet carries no
        // input or output, only the completion event of the session.
        if unsafe { encode(self.core.encoder, &mut *params) } != NV_ENC_SUCCESS {
            return false;
        }
        waited(LATE_WAIT_MS)
    }
}

impl Drop for NvencSession {
    fn drop(&mut self) {
        self.drain_late();
        self.core.release_outputs();
        for entry in std::mem::take(&mut self.registered) {
            unregister(&self.core.api, self.core.encoder, entry);
        }
    }
}
