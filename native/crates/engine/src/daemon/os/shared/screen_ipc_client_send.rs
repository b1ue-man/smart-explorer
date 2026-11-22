//! The control frames a relayed viewer session sends to the worker
//! (`RELAY_VIEWER`), and the frames of the viewer's automatic fallback
//! (`RELAY_VIEWER_FALLBACK`, round 11, N2), which the worker sends on but
//! does not keep as the viewer's choice for a rebuilt session.
use super::*;

impl StreamControl {
    pub fn send(&self, frame: &ViewerFrame) -> io::Result<()> {
        self.send_as(frame, RELAY_VIEWER)
    }

    /// `frame` (a `SET_MODE` with `lossless`) carries only the viewer's
    /// automatic fallback, not its choice.
    pub fn send_fallback(&self, frame: &ViewerFrame) -> io::Result<()> {
        self.send_as(
            frame,
            crate::daemon::ipc_protocol::screen::switch::RELAY_VIEWER_FALLBACK,
        )
    }

    fn send_as(&self, frame: &ViewerFrame, relay: u8) -> io::Result<()> {
        let (tag, payload) = encode_viewer_frame(frame)?;
        let mut body = Vec::with_capacity(payload.len() + 1);
        body.push(tag);
        body.extend_from_slice(&payload);
        let mut input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
        write_relay(&mut *input, relay, &body)
    }
}
