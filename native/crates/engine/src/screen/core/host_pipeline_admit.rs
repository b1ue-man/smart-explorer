//! What may enter the stage before a picture is captured and encoded:
//! a picture encoded ahead of its frame (P9) and the size a chosen video
//! codec carries (review 2026-11-19). The byte budget of the stream is the
//! rate control's (`rate_tiles.rs`).
use crate::screen::core::host_pipeline::{Pipeline, Produced};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::wire_binary::StreamKind;

impl Pipeline {
    /// One picture may be encoded ahead of its frame when the unacknowledged
    /// bytes plus this frame and the next one (each the session's mean frame
    /// size) stay under the queue guard of the pacer (`Pacer::guard`, P9);
    /// a preview never does.
    pub(super) fn ahead_allowed(&self, target: u64, path: PathKind, rtt_us: u64) -> bool {
        if self.kind == StreamKind::Preview || self.totals.frames == 0 {
            return false;
        }
        let mean = self.totals.bytes / self.totals.frames;
        self.pacer
            .unacked_bytes()
            .saturating_add(mean.saturating_mul(2))
            <= self.pacer.guard(target, path, rtt_us)
    }

    /// A chosen video codec waits for a picture of at least 2 x 2 pixels
    /// (plan 2026-11-18; only „Automatisch“ takes the tiles meanwhile).
    /// Review 2026-11-19: it waited without a word; the viewer is told once
    /// per size why no picture comes, through the reason of the next
    /// `STATE` that names the chosen codec (`note_encoder`).
    pub(super) fn wait_for_size(&mut self, fitted: FrameSize) -> Produced {
        if self.quality_policy.small_picture != Some(fitted) {
            self.quality_policy.small_picture = Some(fitted);
            self.note_encoder(format!(
                "chosen_codec_unavailable: {}: the picture of {}x{} pixels is smaller than \
                 the 2x2 pixels the codec needs; the stream waits for a larger one",
                self.quality_policy.scope().key(),
                fitted.width,
                fitted.height
            ));
        }
        Produced::Idle
    }
}
