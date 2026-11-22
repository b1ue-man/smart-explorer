//! The Media Foundation encoder after its GPU path failed: NV12 in system
//! memory from the next picture (split from `mf_encoder.rs`, review
//! 2026-11-19 round 9).
use super::*;

impl AsyncEncoder {
    /// The GPU path failed: CPU path for the session, this encoder on NV12
    /// in system memory; else the original error (the next encoder follows).
    pub(super) fn gpu_failed(&mut self, error: io::Error) -> io::Result<CapturedEncodedFrame> {
        // A device of its own is not the capture's (P3).
        if self.gpu.as_ref().is_some_and(|gpu| !gpu.is_own()) {
            d3d_input::disable(&format!("{}: {error}", self.setup.name));
        }
        self.gpu = None;
        if self.restart_system().is_err() {
            return Err(error);
        }
        self.refresh = true;
        self.restarted = true;
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "the hardware encoder takes system memory from the next picture",
        ))
    }

    /// Ends the stream, takes the device manager back and starts anew.
    pub(super) fn restart_system(&mut self) -> io::Result<()> {
        stop(&self.transform);
        // SAFETY: messages and type resets on the activated transform; the
        // types are set again by `start`.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.transform.SetInputType(0, None::<&IMFMediaType>, 0);
            let _ = self.transform.SetOutputType(0, None::<&IMFMediaType>, 0);
        }
        d3d_input::detach(&self.transform);
        // Events of the old stream give no credit for the new one.
        for _ in 0..64 {
            // SAFETY: a non-blocking poll of the transform's event queue.
            if unsafe { self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) }.is_err() {
                break;
            }
        }
        self.credits = 0;
        start(&self.transform, &self.setup, self.bitrate)?;
        self.sequence_header = mf_capture_output::sequence_header(&self.transform);
        Ok(())
    }
}
