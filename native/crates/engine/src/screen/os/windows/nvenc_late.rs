//! A picture NVENC did not complete in time (review 2026-11-19, Gemini
//! round 3 finding 14). `nvEncodeAPI.h` 12.1: the client unmaps an input
//! only "after ::NvEncLockBitstream() API returns successfully for encode
//! work submitted using the mapped input buffer" (`NvEncUnmapInputResource`),
//! and before `NvEncDestroyEncoder` it "must flush the encoder ... pass a
//! NULL encode picture packet and ... wait for the flush event to be
//! signaled by the encoder in asynchronous mode". A late picture therefore
//! keeps its input mapped (and its capture ring slot, which `GpuInput`
//! gives back after the session went, `VendorEncoder` drops the session
//! first); the session flushes with the end-of-stream packet, waits for it
//! at most `FLUSH_WAIT`, then unmaps, and only then goes.
use super::*;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

/// `NV_ENC_PIC_FLAG_EOS` of `nvEncodeAPI.h`: the end of the input stream.
const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;
/// Longest wait for the flush of a session with a late picture; a driver
/// that never answers still lets the session go.
const FLUSH_WAIT_MS: u32 = 2_000;

impl NvencSession {
    /// The mapped input of a picture that timed out stays mapped.
    pub(super) fn keep_late(&mut self, mapped: Ptr) {
        self.late = Some(mapped);
    }

    /// Before the session goes: flush, wait, unmap the late input.
    pub(super) fn drain_late(&mut self) {
        let Some(mapped) = self.late.take() else {
            return;
        };
        if let Some(encode) = self.core.api.table().encode_picture {
            let mut params = Box::new(NvEncPicParams::zeroed());
            params.version = NV_ENC_PIC_PARAMS_VER;
            params.encode_pic_flags = NV_ENC_PIC_FLAG_EOS;
            params.completion_event = self.core.event;
            // SAFETY: valid async session; the end-of-stream packet carries
            // no input or output, only the completion event of the session.
            let result = unsafe { encode(self.core.encoder, &mut *params) };
            if result == NV_ENC_SUCCESS {
                // SAFETY: the session's live completion event.
                let waited = unsafe { WaitForSingleObject(self.core.event, FLUSH_WAIT_MS) };
                if waited != WAIT_OBJECT_0 {
                    eprintln!("screen encoder nvenc: flush of a late picture not signalled");
                }
            }
        }
        if let Some(unmap) = self.core.api.functions.unmap_input_resource {
            // SAFETY: mapped by this session; its work completed or the
            // session is ending without it.
            unsafe { unmap(self.core.encoder, mapped) };
        }
    }
}
