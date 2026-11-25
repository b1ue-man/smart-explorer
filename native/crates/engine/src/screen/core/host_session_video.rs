//! The host's frames of the picture carriers outside `HostFrame`
//! (`video_transport.rs`): `VIDEO_ANSWER`, `VIDEO_TCP_ANSWER` and
//! `VIDEO_TCP_RESULT` as the link tasks queue them, and `VIDEO_TAIL` once a
//! picture sent as datagrams is the last before a still screen
//! (`video_host::VideoHost::next_frame`). Sent on every capture tick and
//! with the statistics; the viewer's control reader takes them before the
//! host frames are decoded. Each goes whole and never inside a large frame
//! being written piece by piece.
use super::*;

impl Session<'_> {
    pub(super) async fn video_frames(&mut self) -> io::Result<()> {
        while !self.outbox.mid_frame() {
            let Some((tag, payload)) = self.video.next_frame() else {
                return Ok(());
            };
            let peer = self.connection.remote_id().to_string();
            let sent = control_sender::send_raw(&peer, &mut self.send, tag, &payload).await?;
            self.pipeline.book_control_bytes(sent);
        }
        Ok(())
    }
}
