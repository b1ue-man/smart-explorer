//! Masking of a byte stream (command output). The masker never emits a
//! part of a possible match before the match is complete: it holds back the
//! tail that a match could still reach (the matcher's hold window) and cuts
//! only before a word, never inside a match. At the end of the stream
//! (`flush`) and after [`STREAM_IDLE_RELEASE`] without new output
//! (`release_idle`) it releases the rest, so a prompt without a line ending
//! still reaches the agent.
//!
//! Limit: a regex entry's match longer than the regex window (64 KiB) that
//! is still growing at the cut is emitted in two parts.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fold::is_word_char;
use super::mask_set::MaskSet;
use super::matcher::char_before;

/// Silence after which held output is released.
pub const STREAM_IDLE_RELEASE: Duration = Duration::from_millis(400);
/// Held bytes beyond the hold window after which the masker cuts even
/// inside an unbroken run of word characters.
const MAX_EXTRA_PENDING: usize = 4 * 1024 * 1024;
/// Smallest batch beyond the hold window that triggers a scan.
const MIN_BATCH: usize = 4096;

/// Masks a stream with one [`MaskSet`].
pub struct StreamMasker {
    set: Arc<MaskSet>,
    pending: Vec<u8>,
    last_push: Option<Instant>,
}

impl StreamMasker {
    pub(crate) fn new(set: Arc<MaskSet>) -> Self {
        Self {
            set,
            pending: Vec::new(),
            last_push: None,
        }
    }

    /// Takes more output; returns what can be shown now.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.last_push = Some(Instant::now());
        if self.set.is_empty() {
            return bytes.to_vec();
        }
        self.pending.extend_from_slice(bytes);
        let hold = self.set.hold();
        // Scan only once a batch beyond the hold window arrived, so many
        // small pieces do not rescan the window each time.
        if self.pending.len() < hold + (hold / 4).max(MIN_BATCH) {
            return Vec::new();
        }
        let spans = self.set.find_spans(&self.pending);
        let mut cut = word_cut(&self.pending, self.pending.len() - hold);
        loop {
            let inside = spans
                .iter()
                .find(|span| span.start < cut && cut < span.end)
                .map(|span| span.start);
            match inside {
                Some(start) => cut = word_cut(&self.pending, start),
                None => break,
            }
        }
        if cut == 0 {
            if self.pending.len() <= hold + MAX_EXTRA_PENDING {
                return Vec::new();
            }
            cut = char_cut(&self.pending, self.pending.len() - hold);
        }
        let before: Vec<_> = spans.into_iter().filter(|span| span.end <= cut).collect();
        let out = self.set.render(&self.pending[..cut], &before).0;
        self.pending.drain(..cut);
        out
    }

    /// End of the stream: masks and returns everything held.
    pub fn flush(&mut self) -> Vec<u8> {
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return pending;
        }
        let spans = self.set.find_spans(&pending);
        self.set.render(&pending, &spans).0
    }

    /// Releases the held rest once the stream was silent for
    /// [`STREAM_IDLE_RELEASE`]; otherwise returns nothing.
    pub fn release_idle(&mut self) -> Vec<u8> {
        self.release_idle_at(Instant::now())
    }

    /// [`StreamMasker::release_idle`] with an explicit clock.
    pub fn release_idle_at(&mut self, now: Instant) -> Vec<u8> {
        match self.last_push {
            Some(last) if now.saturating_duration_since(last) >= STREAM_IDLE_RELEASE => {
                self.flush()
            }
            _ => Vec::new(),
        }
    }

    /// When the held rest is released if no output follows.
    pub fn idle_deadline(&self) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        self.last_push.map(|last| last + STREAM_IDLE_RELEASE)
    }

    /// Bytes held back right now.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// The last position at or before `pos` that starts a new word: a
/// character boundary whose preceding character is not a word character.
fn word_cut(bytes: &[u8], pos: usize) -> usize {
    let mut pos = pos.min(bytes.len());
    while pos > 0 {
        let boundary = pos == bytes.len() || !is_continuation(bytes[pos]);
        if boundary && !char_before(bytes, pos).is_some_and(is_word_char) {
            return pos;
        }
        pos -= 1;
    }
    0
}

/// The last character boundary at or before `pos`.
fn char_cut(bytes: &[u8], pos: usize) -> usize {
    let mut pos = pos.min(bytes.len());
    while pos > 0 && pos < bytes.len() && is_continuation(bytes[pos]) {
        pos -= 1;
    }
    pos
}

fn is_continuation(byte: u8) -> bool {
    (0x80..=0xBF).contains(&byte)
}
