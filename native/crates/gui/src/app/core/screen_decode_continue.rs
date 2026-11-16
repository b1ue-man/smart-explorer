//! The decode thread's decoder chain (plan 2026-11-14 „Bildkette“ B-36,
//! handed over by W3) and the decoder choice without a new session (V1).
//!
//! - A newer picture stream that starts with a continuation
//!   (`FLAG_CONTINUE`) keeps the decoder and its references when the chain
//!   is intact (`DecoderContinuity`) and the decoder can follow it
//!   (`ScreenDecoder::keeps_references`). Otherwise the stream is skipped,
//!   the last good picture stays and one key frame is asked for on the
//!   existing way (`KEYFRAME_REQUEST`).
//! - A stream that broke off leaves the decoder in place while the session
//!   announced `gop_continue` and the chain is intact: the host's next
//!   stream (continuation or key frame) follows. Asking for a key frame at
//!   once would reach the host before it opened that stream and spoil the
//!   continuation. Without a new stream within `STALE_STREAM_US` the
//!   viewer asks after all.
//! - A changed „Dekodierer“ (`ViewerSession::switch_decoder`) asks for a
//!   key frame; the old decoder decodes until it arrives, then the decoder
//!   of the new choice opens for it.
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::*;
use crate::screen::core::gop_continue::StreamStart;
use crate::screen::core::pacing::STALE_STREAM_US;

impl DecodeState {
    /// Whether the session announced a continuation (`gop_continue` or
    /// `gop_continue_decoders` of `StreamRequest`); a later `SET_DECODERS`
    /// replaces it (`ViewerSession::set_decoders`, P1).
    pub fn announce_continuation(&mut self, on: bool) {
        self.shared.local.continuation.store(on, Ordering::Release);
    }

    /// The first record of a newer picture stream; `false` when the stream
    /// is skipped. Only a continuation is decided here: a key frame opens a
    /// decoder and any other start fails in its new decoder, as before.
    pub(super) fn stream_start(&mut self, header: &GopHeader, record: &FrameRecord) -> bool {
        self.chain.awaiting = None;
        if !record.is_continuation() {
            if record.is_keyframe() {
                self.chain.continuity.start(record);
            } else {
                self.chain.continuity.broken();
            }
            return true;
        }
        match self.chain.continuity.start(record) {
            StreamStart::OpenDecoder => true,
            StreamStart::KeepDecoder
                if self.keeps_decoder()
                    && self
                        .gop
                        .as_ref()
                        .is_some_and(|gop| gop.matches_continuation(header)) =>
            {
                if let Some(gop) = self.gop.as_mut() {
                    gop.id = header.gop_id;
                    gop.header = *header;
                }
                true
            }
            StreamStart::KeepDecoder | StreamStart::RequestKeyframe => {
                self.skip_stream(header.gop_id, record.frame_id);
                false
            }
        }
    }

    /// The running decoder may go on over a continuation.
    fn keeps_decoder(&self) -> bool {
        !self.chain.reopen
            && self
                .gop
                .as_ref()
                .is_some_and(|gop| gop.decoder.keeps_references())
    }

    /// A stream whose start cannot be decoded: the shown picture stays
    /// (without receipts, input blocked) until the requested key frame.
    fn skip_stream(&mut self, gop_id: u64, frame_id: u64) {
        self.chain.continuity.broken();
        self.latest_gop = gop_id;
        self.gop = None;
        self.shared.suspend_presentation();
        self.broken = Some(gop_id);
        self.broken_frame = Some(frame_id);
        self.request_keyframe(KeyframeReason::Reset, gop_id);
    }

    /// `Broken` of the newest stream: `true` when the decoder waits for the
    /// host's next stream instead of asking for a key frame now.
    pub(super) fn waits_for_stream(&mut self, gop_id: u64) -> bool {
        let partial =
            self.chain.open_frame && self.last_record.is_some_and(|(group, _)| group == gop_id);
        if partial {
            // The decoder holds part of a frame: no continuation follows it.
            self.chain.continuity.broken();
        }
        // A newer stream that broke before one of its records reached the
        // decoder (a continuation that went stale in turn) leaves the chain
        // as it was: the decoder waits on for the host's next stream.
        let wait = self.shared.local.continuation.load(Ordering::Acquire)
            && self.chain.continuity.intact()
            && self.gop.as_ref().is_some_and(|gop| gop.id <= gop_id)
            && self.keeps_decoder();
        if wait {
            // Streams that break one after another share one wait: without
            // a record within `STALE_STREAM_US` the key frame is asked for.
            let since = self
                .chain
                .awaiting
                .map_or_else(Instant::now, |(_, since)| since);
            self.chain.awaiting = Some((gop_id, since));
        } else {
            self.chain.continuity.broken();
        }
        wait
    }

    /// On every event: a broken stream without a successor within
    /// `STALE_STREAM_US` asks for the key frame after all.
    pub(super) fn check_awaiting(&mut self) {
        let Some((gop_id, since)) = self.chain.awaiting else {
            return;
        };
        if since.elapsed() < Duration::from_micros(STALE_STREAM_US) {
            return;
        }
        self.chain.awaiting = None;
        self.chain.continuity.broken();
        self.gop = None;
        self.request_keyframe(KeyframeReason::Reset, gop_id);
    }

    /// V1: a decoder choice taken over without a new session asks for the
    /// key frame the new decoder starts with.
    pub(super) fn poll_switch(&mut self) {
        let switches = self.shared.local.switches.load(Ordering::Acquire);
        if switches == self.chain.switches {
            return;
        }
        self.chain.switches = switches;
        self.chain.reopen = true;
        self.chain.continuity.broken();
        self.request_keyframe(KeyframeReason::Reset, self.latest_gop);
    }

    /// The key frame that opens the decoder of a new choice (V1).
    pub(super) fn reopens_at(&mut self, keyframe: bool, first_of_frame: bool) -> bool {
        keyframe && first_of_frame && std::mem::take(&mut self.chain.reopen)
    }
}

/// The decode thread's state of its decoder chain.
#[derive(Default)]
pub(super) struct Chain {
    pub(super) continuity: crate::screen::core::gop_continue::DecoderContinuity,
    /// The newest stream broke off with an intact chain at that time.
    pub(super) awaiting: Option<(u64, Instant)>,
    /// The last record handed to the decoder left its frame open.
    pub(super) open_frame: bool,
    /// Decoder switches taken over (`DecodeLocal::switches`).
    pub(super) switches: u64,
    /// The next key frame opens a decoder of the new choice.
    pub(super) reopen: bool,
}

#[cfg(test)]
#[path = "bildkette_viewer_task_tests.rs"]
mod bildkette_viewer_task_tests;
