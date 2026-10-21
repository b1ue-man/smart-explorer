//! The viewer's side of the binary path on Android (SCR-M9, U28-4): the
//! queue between the relayed session and `screenPull`, and the picture-group
//! logic in front of it. H.264 access units go to the app unchanged (Annex B,
//! SPS/PPS before every IDR); tile frames are composed here to a finished
//! picture in byte order R,G,B,A and only the newest one is kept. A queue
//! whose oldest picture is older than `STALL_US` is emptied and a `Reset`
//! key frame is requested (the rule of C3 (4)); until the new key frame
//! arrives the app gets nothing. Nothing of a picture is ever logged.
//!
//! Layout of a pull, little endian: bytes 0–7 `pts_us` (the host's capture
//! time), 8–11 `flags`, 12–15 `len`, then `len` payload bytes. Flags: bit 0
//! key frame, bit 1 finished tile picture (R,G,B,A), bit 2 first picture of
//! a new picture group, bit 3 the access unit is complete (always set);
//! bits 16–31 are the low 16 bits of the group number that the matching
//! `stream` event names.
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::screen::core::host_backend::{ScreenDecoder, TilesScreenDecoder};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::KeyframeRequester;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader};
use crate::screen::core::wire_control::{KeyframeReason, KeyframeRequest};

pub(crate) const HEADER_BYTES: usize = 16;
pub(crate) const FLAG_KEYFRAME: u32 = 0x1;
/// A finished tile picture; the payload is `width × height × 4` bytes R,G,B,A
/// (the flag keeps the name of the contract).
pub(crate) const FLAG_TILES_BGRA: u32 = 0x2;
pub(crate) const FLAG_NEW_STREAM: u32 = 0x4;
pub(crate) const FLAG_LAST_OF_PICTURE: u32 = 0x8;
pub(crate) const GOP_SHIFT: u32 = 16;
/// `screenPull` answers.
pub(crate) const PULL_TIMEOUT: i32 = -1;
pub(crate) const PULL_ENDED: i32 = -2;
pub(crate) const PULL_TOO_SMALL: i32 = -3;
pub(crate) const PULL_BRIDGE_ERROR: i32 = -4;
/// Longest wait of one pull: a pull blocks a decoding thread, and a shorter
/// wait lets it notice the close in time.
pub(crate) const MAX_PULL_WAIT: Duration = Duration::from_millis(1000);
/// Age of the oldest queued picture after which the group is reset (C3 (4)).
pub(crate) const STALL_US: u64 = 1_000_000;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Queued {
    pts_us: u64,
    flags: u32,
    data: Vec<u8>,
    queued_us: u64,
}

#[derive(Default)]
struct QueueState {
    items: VecDeque<Queued>,
    waiting_keyframe: bool,
    ended: bool,
}

/// What `PullQueue::push` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pushed {
    pub queued: bool,
    /// The queue was emptied because the app pulled too slowly.
    pub stalled: bool,
}

/// The pictures between the session and the app's decoding thread.
#[derive(Default)]
pub(crate) struct PullQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

impl PullQueue {
    /// Queues a picture; `now_us` is the clock of the stall rule.
    pub(crate) fn push(&self, pts_us: u64, mut flags: u32, data: Vec<u8>, now_us: u64) -> Pushed {
        let mut state = lock(&self.state);
        let none = |stalled| Pushed {
            queued: false,
            stalled,
        };
        if state.ended {
            return none(false);
        }
        let tiles = flags & FLAG_TILES_BGRA != 0;
        let mut stalled = false;
        if !tiles
            && state
                .items
                .front()
                .is_some_and(|front| now_us.saturating_sub(front.queued_us) > STALL_US)
        {
            state.items.clear();
            state.waiting_keyframe = true;
            stalled = true;
        }
        if flags & FLAG_NEW_STREAM != 0 {
            state.items.clear();
        }
        if flags & FLAG_KEYFRAME != 0 {
            state.waiting_keyframe = false;
        } else if !tiles && state.waiting_keyframe {
            return none(stalled);
        }
        if tiles {
            // Only the newest tile picture is kept; it keeps the mark of the
            // group start the app has not seen yet.
            let mut inherited = 0;
            state.items.retain(|old| {
                if old.flags & FLAG_TILES_BGRA == 0 {
                    return true;
                }
                inherited |= old.flags & FLAG_NEW_STREAM;
                false
            });
            flags |= inherited;
        }
        state.items.push_back(Queued {
            pts_us,
            flags,
            data,
            queued_us: now_us,
        });
        self.ready.notify_one();
        Pushed {
            queued: true,
            stalled,
        }
    }

    /// Writes the oldest picture into `dst` (layout above) and returns the
    /// bytes written, or one of the `PULL_*` codes. Too small a `dst` leaves
    /// the picture queued and, when it holds 16 bytes, names the payload
    /// length in bytes 12–15.
    pub(crate) fn pull_into(&self, dst: &mut [u8], timeout: Duration) -> i32 {
        let deadline = Instant::now() + timeout.min(MAX_PULL_WAIT);
        let mut state = lock(&self.state);
        loop {
            if let Some(front) = state.items.front() {
                let len = front.data.len();
                let (Ok(wire_len), Ok(total)) = (
                    u32::try_from(len),
                    i32::try_from(len.saturating_add(HEADER_BYTES)),
                ) else {
                    return PULL_BRIDGE_ERROR;
                };
                if dst.len() < HEADER_BYTES + len {
                    if dst.len() >= HEADER_BYTES {
                        dst[12..16].copy_from_slice(&wire_len.to_le_bytes());
                    }
                    return PULL_TOO_SMALL;
                }
                dst[0..8].copy_from_slice(&front.pts_us.to_le_bytes());
                dst[8..12].copy_from_slice(&front.flags.to_le_bytes());
                dst[12..16].copy_from_slice(&wire_len.to_le_bytes());
                dst[HEADER_BYTES..HEADER_BYTES + len].copy_from_slice(&front.data);
                state.items.pop_front();
                return total;
            }
            if state.ended {
                return PULL_ENDED;
            }
            let now = Instant::now();
            if now >= deadline {
                return PULL_TIMEOUT;
            }
            state = self
                .ready
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Ends the queue: it is emptied and every pull answers `PULL_ENDED`.
    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.ended = true;
        state.items.clear();
        self.ready.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        lock(&self.state).items.len()
    }
}

/// What the session thread has to do after a record.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SinkOutput {
    /// A new picture group starts: the `stream` event.
    NewStream {
        gop: u64,
        codec: CodecKind,
        profile: Option<H264Profile>,
        size: FrameSize,
    },
    Keyframe(KeyframeRequest),
}

struct Gop {
    id: u64,
    tiles: Option<TilesScreenDecoder>,
}

/// Turns the records of the relayed session into queued pictures.
pub(crate) struct ViewerSink {
    queue: Arc<PullQueue>,
    keyframes: Arc<Mutex<KeyframeRequester>>,
    gop: Option<Gop>,
    /// The newest group seen; records of older ones are stale.
    latest_gop: u64,
    /// A group whose records are skipped until its next key frame.
    broken: Option<u64>,
    /// The next queued picture starts a group for the app.
    new_stream: bool,
}

impl ViewerSink {
    pub(crate) fn new(queue: Arc<PullQueue>, keyframes: Arc<Mutex<KeyframeRequester>>) -> Self {
        Self {
            queue,
            keyframes,
            gop: None,
            latest_gop: 0,
            broken: None,
            new_stream: false,
        }
    }

    /// At most one key frame request is open (`KEYFRAME_RETRY_US`).
    fn request(&self, reason: KeyframeReason, gop_id: u64, now_us: u64) -> Option<SinkOutput> {
        let due = lock(&self.keyframes).should_request(now_us);
        due.then_some(SinkOutput::Keyframe(KeyframeRequest {
            reason,
            gop_id: Some(gop_id),
        }))
    }

    /// The relay reports a broken picture stream of group `gop_id`.
    pub(crate) fn broken(&mut self, gop_id: u64, now_us: u64) -> Option<SinkOutput> {
        if self.gop.as_ref().is_some_and(|gop| gop.id == gop_id) {
            self.gop = None;
        }
        self.broken = Some(gop_id);
        self.request(KeyframeReason::Reset, gop_id, now_us)
    }

    /// Applies one record of group `header`.
    pub(crate) fn record(
        &mut self,
        header: &GopHeader,
        record: &FrameRecord,
        now_us: u64,
    ) -> Vec<SinkOutput> {
        let mut out = Vec::new();
        if header.gop_id < self.latest_gop {
            return out;
        }
        let keyframe = record.is_keyframe();
        if self.broken == Some(header.gop_id) {
            if !keyframe {
                return out;
            }
            self.broken = None;
            self.gop = None;
        }
        if self.gop.as_ref().is_none_or(|gop| gop.id != header.gop_id) {
            let tiles = match header.codec {
                CodecKind::H264 => None,
                CodecKind::Tiles => match TilesScreenDecoder::new(header.size) {
                    Ok(decoder) => Some(decoder),
                    Err(_) => {
                        self.broken = Some(header.gop_id);
                        out.extend(self.request(
                            KeyframeReason::DecodeError,
                            header.gop_id,
                            now_us,
                        ));
                        return out;
                    }
                },
            };
            self.gop = Some(Gop {
                id: header.gop_id,
                tiles,
            });
            self.latest_gop = header.gop_id;
            self.new_stream = true;
            out.push(SinkOutput::NewStream {
                gop: header.gop_id,
                codec: header.codec,
                profile: header.profile,
                size: header.size,
            });
        }
        let mut flags = if keyframe { FLAG_KEYFRAME } else { 0 };
        let data = match self.gop.as_mut().and_then(|gop| gop.tiles.as_mut()) {
            None if record.has_more() => return out,
            None => record.payload.clone(),
            Some(decoder) => match decoder.decode(record) {
                Ok(true) => {
                    flags |= FLAG_TILES_BGRA;
                    bgra_to_rgba(decoder.picture().data())
                }
                Ok(false) => return out,
                Err(_) => {
                    self.broken = Some(header.gop_id);
                    out.extend(self.request(KeyframeReason::DecodeError, header.gop_id, now_us));
                    return out;
                }
            },
        };
        if keyframe {
            lock(&self.keyframes).on_keyframe();
        }
        let gop_bits = ((header.gop_id & 0xFFFF) as u32) << GOP_SHIFT;
        flags |= FLAG_LAST_OF_PICTURE | gop_bits;
        if self.new_stream {
            flags |= FLAG_NEW_STREAM;
        }
        let pushed = self.queue.push(record.capture_ts_us, flags, data, now_us);
        if pushed.queued {
            self.new_stream = false;
        }
        if pushed.stalled {
            self.new_stream = true;
            out.extend(self.request(KeyframeReason::Reset, header.gop_id, now_us));
        }
        out
    }
}

/// The byte order Android's `ARGB_8888` bitmaps take from a buffer.
pub(crate) fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut out = bgra.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    out
}
