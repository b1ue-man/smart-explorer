//! The input side of a screen source (`ScreenInputSink`). Split from
//! `host_backend.rs` (round 12 rest: the input receipts of a sink that
//! queues its batches for later, `input_mark`/`input_outcome`).
use super::*;

/// Release-only endpoint belonging to one existing source.
pub trait ScreenInputSink: Send + Sync {
    fn legacy_input_invalid(&self) -> bool {
        false
    }
    fn inject(
        &self,
        _events: &[InputEvent],
        _reference_generation: u64,
        _presented: bool,
    ) -> Result<(), ControlBlock> {
        Err(ControlBlock::ControlUnavailable)
    }
    fn release(&self, events: &[InputEvent]) -> Result<(), ControlBlock>;
    /// The last release of a session that gives the control up or ends
    /// (`host_input_release.rs`, `drain_releases`): a sink that queues input
    /// for later (Android) also takes back what of this session still waits;
    /// every other sink lets go as with `release`.
    fn revoke(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        self.release(events)
    }
    /// A batch `inject` accepted that the platform refused afterwards (an
    /// Android gesture, a Wayland portal call): the session reports it as a
    /// block once (review 2026-11-19). `None` for sinks that answer in
    /// `inject` itself.
    fn take_refusal(&self) -> Option<ControlBlock> {
        None
    }
    /// Input receipts (`input_receipt.rs`): the number of the last batch a
    /// sink that queues input for later (Android: its app performs it) took
    /// in `inject`; `None` for a sink that performs a batch before `inject`
    /// returns.
    fn input_mark(&self) -> Option<u64> {
        None
    }
    /// What became of the batches this sink took after mark `after` up to
    /// mark `mark`: `None` while the platform has not finished all of them,
    /// `Some(None)` when it performed them, `Some(Some(sentence))` when it
    /// refused one (the device's sentence, `codec_reason_text`).
    fn input_outcome(&self, _after: u64, _mark: u64) -> Option<Option<String>> {
        None
    }
    /// The sink can take no input any more (its host process ended): a
    /// keeper of releases stops trying at once (round 8, I4).
    fn closed(&self) -> bool {
        false
    }
    /// The input target this sink injects into: sessions with the same
    /// target wait for each other's releases (round 8, I4). One desktop per
    /// host by default: Windows' `SendInput` reaches the interactive desktop
    /// whatever monitor or window the source shows, X11 one display, the
    /// Wayland portal one seat of the login, Android one device; the host
    /// processes all follow the one active session (`session_target`).
    fn input_target(&self) -> String {
        "desktop".into()
    }
    /// Takes the pointer out of the picture (`true`) or back into it (P6,
    /// `cursor_overlay.rs`); `false` while this source does not separate it
    /// (it cannot, or its capture has not started: the wish is kept).
    /// Blocks while the source answers: call it off the async runtime.
    fn set_cursor_overlay(&self, _on: bool) -> bool {
        false
    }
    /// Where the separated pointer arrives, for a source that can.
    fn cursor_feed(
        &self,
    ) -> Option<std::sync::Arc<crate::screen::core::cursor_overlay::CursorFeed>> {
        None
    }
    fn family_snapshot(&self) -> io::Result<Option<crate::remote_window::FamilySnapshot>> {
        Ok(None)
    }
    fn family_operation(
        &self,
        _request: crate::screen::core::family_protocol::FamilyOperation,
        _active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> io::Result<crate::screen::core::family_protocol::FamilyOperationResult> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source_has_no_window_family",
        ))
    }
    fn family_input(
        &self,
        _request: crate::screen::core::family_protocol::FamilyInput,
        _active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> io::Result<crate::screen::core::family_protocol::FamilyInputResult> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source_has_no_window_family",
        ))
    }
}
