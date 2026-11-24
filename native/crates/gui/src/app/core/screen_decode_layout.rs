//! A window family's new layout on the desktop viewer (round 12 rest,
//! Codex 12/8): the records of a group the host opened for a layout newer
//! than the family state the viewer holds wait in `layout_hold` until that
//! state arrives (`screen_decode_events.rs`) and are decoded then, the key
//! frame first; before, they were dropped and the viewer found back only
//! over a decode error and one more key frame request (one more group).
//! Records of an older layout stay dropped (`family_layout_hold.rs`).
use super::*;
use crate::screen::core::family_layout_hold::{fit, LayoutFit};

type Dropped = super::super::screen_chain_window::flow::Dropped;

impl DecodeState {
    /// The layout check of a record: `Some` to decode it now, `None` when it
    /// waits for its layout state or was dropped.
    pub(super) fn layout_gate(
        &mut self,
        header: GopHeader,
        record: FrameRecord,
    ) -> Option<FrameRecord> {
        // Read under the lock and released before counting: `left_out` takes
        // the same lock (Codex 3/1: a deadlock of the decode thread).
        let known = {
            let info = self.shared.info();
            let Some(family) = info.family.as_ref() else {
                return Some(record);
            };
            family
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.layout_revision)
        };
        match fit(header.family_layout_revision, known) {
            LayoutFit::Decode => Some(record),
            LayoutFit::Drop => {
                self.drop_held(&record);
                None
            }
            LayoutFit::Hold => {
                let held = self.layout_hold.hold(header, record);
                for (_, lost) in &held.replaced {
                    self.drop_held(lost);
                }
                None
            }
        }
    }

    /// A family state arrived: the records held for its layout are decoded
    /// now, in their order, and those of an older layout are dropped.
    /// `true` when records were decoded.
    pub(super) fn release_layout(&mut self) -> bool {
        let known = self
            .shared
            .info()
            .family
            .as_ref()
            .and_then(|family| family.snapshot.as_ref())
            .map(|snapshot| snapshot.layout_revision);
        let Some(known) = known else {
            return false;
        };
        let (decode, dropped) = self.layout_hold.release(known);
        for (_, record) in &dropped {
            self.drop_held(record);
        }
        let released = !decode.is_empty();
        for (header, record) in decode {
            self.record(header, record);
        }
        released
    }

    /// A record left out for its layout: counted as received and not
    /// decoded, and the decoder chain is broken (B-36).
    fn drop_held(&mut self, record: &FrameRecord) {
        if !record.has_more() {
            self.flow(|flow| flow.received());
        }
        self.chain.continuity.broken();
        self.left_out(record, Dropped::NotDecoded);
    }
}

#[cfg(test)]
#[path = "screen_decode_layout_task_tests.rs"]
mod tests;
