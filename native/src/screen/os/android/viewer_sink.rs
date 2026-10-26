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
//! bits 16–31 retain the low 16 group bits as a legacy hint only. Flag 0x100
//! prefixes the payload with the full little-endian `gop_id u64`; flag 0x80
//! follows it with `frame_id u64`. Every ViewerSink output sets both flags,
//! including v1 wire streams. Only the full GOP identifies its `stream` event.
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
pub(crate) const FLAG_HYBRID: u32 = 0x10;
pub(crate) const FLAG_MORE_PARTS: u32 = 0x20;
pub(crate) const FLAG_FRAME_ID: u32 = 0x80;
pub(crate) const FLAG_GOP_ID: u32 = 0x100;
const FLAG_HYBRID_BEGIN: u32 = 0x40;
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
    bytes: usize,
    waiting_keyframe: bool,
    ended: bool,
}
impl QueueState {
    fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
    }
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
    frame_bytes: std::sync::atomic::AtomicUsize,
    frame_items: std::sync::atomic::AtomicUsize,
}

impl PullQueue {
    fn set_frame_limits(
        &self,
        limits: crate::screen::core::frame_budget::FrameLimits,
        size: FrameSize,
        contract: Option<crate::screen::core::frame_budget::FrameContract>,
        admitted: bool,
    ) -> std::io::Result<()> {
        limits.check(size)?;
        let admitted = admitted
            && (contract.is_some()
                || limits.working_bytes < crate::screen::core::frame_budget::MAX_WORKING_BYTES);
        let budget = crate::screen::core::frame_size_caps::PacketBudget::for_receive(
            contract,
            admitted.then_some(limits),
        )?;
        let bound = budget.encoded_bytes;
        let bound = if contract.is_none() && !admitted {
            bound.max(64 * 1024 * 1024)
        } else {
            bound
        };
        self.frame_bytes
            .store(bound, std::sync::atomic::Ordering::Release);
        self.frame_items.store(
            if contract.is_some() || admitted {
                budget.records
            } else {
                0
            },
            std::sync::atomic::Ordering::Release,
        );
        Ok(())
    }
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
        let configured = self.frame_bytes.load(std::sync::atomic::Ordering::Acquire);
        let bound = if configured == 0 {
            64 * 1024 * 1024
        } else {
            configured
        };
        let slots = self.frame_items.load(std::sync::atomic::Ordering::Acquire);
        let slots = if slots == 0 { usize::MAX } else { slots };
        if data.len() > bound {
            state.clear();
            state.waiting_keyframe = true;
            return none(true);
        }
        if flags & FLAG_NEW_STREAM != 0 {
            state.clear();
        }
        let started = Instant::now();
        // Backpressure preserves a complete lossless transaction when the app
        // consumes its dirty records more slowly than the network receives them.
        while !tiles
            && (state.items.len() >= slots || state.bytes.saturating_add(data.len()) > bound)
        {
            let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            let age = state.items.front().map_or(0, |front| {
                now_us
                    .saturating_add(elapsed)
                    .saturating_sub(front.queued_us)
            });
            if age >= STALL_US {
                break;
            }
            state = self
                .ready
                .wait_timeout(state, Duration::from_micros(STALL_US - age))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            if state.ended {
                return none(false);
            }
        }
        let now_us =
            now_us.saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        if (!tiles
            && (state.items.len() >= slots || state.bytes.saturating_add(data.len()) > bound))
            || (!tiles
                && state
                    .items
                    .front()
                    .is_some_and(|front| now_us.saturating_sub(front.queued_us) > STALL_US))
        {
            state.clear();
            state.waiting_keyframe = true;
            stalled = true;
        }
        if flags & FLAG_NEW_STREAM != 0 {
            state.clear();
        }
        let begins = flags & FLAG_HYBRID == 0 || flags & FLAG_HYBRID_BEGIN != 0;
        if flags & FLAG_KEYFRAME != 0 && begins {
            state.waiting_keyframe = false;
        } else if !tiles && state.waiting_keyframe {
            return none(stalled);
        }
        if tiles {
            // Only the newest tile picture is kept; it keeps the mark of the
            // group start the app has not seen yet.
            let mut inherited = 0;
            let mut removed = 0usize;
            state.items.retain(|old| {
                if old.flags & FLAG_TILES_BGRA == 0 {
                    return true;
                }
                inherited |= old.flags & FLAG_NEW_STREAM;
                removed = removed.saturating_add(old.data.len());
                false
            });
            state.bytes = state.bytes.saturating_sub(removed);
            flags |= inherited;
        }
        state.bytes = state.bytes.saturating_add(data.len());
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
                state.bytes = state.bytes.saturating_sub(len);
                self.ready.notify_all();
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
        state.clear();
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
    tiles: Option<Box<dyn ScreenDecoder>>,
    hybrid: bool,
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
    broken_frame: Option<u64>,
    /// The next queued picture starts a group for the app.
    new_stream: bool,
    local_limits: Option<crate::screen::core::frame_budget::FrameLimits>,
}

impl ViewerSink {
    pub(crate) fn new(queue: Arc<PullQueue>, keyframes: Arc<Mutex<KeyframeRequester>>) -> Self {
        Self {
            queue,
            keyframes,
            gop: None,
            latest_gop: 0,
            broken: None,
            broken_frame: None,
            new_stream: false,
            local_limits: None,
        }
    }
    pub(crate) fn bind_local_limits(
        &mut self,
        limits: Option<crate::screen::core::frame_budget::FrameLimits>,
    ) {
        self.local_limits = limits;
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
        if gop_id < self.latest_gop {
            return None;
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
        let admitted = crate::screen::core::frame_size_caps::PacketBudget::for_receive(
            header.frame_contract,
            self.local_limits,
        )
        .map_or(0, |budget| budget.record_bytes);
        if record.payload.len() > header.record_limit().min(admitted) {
            self.broken = Some(header.gop_id);
            self.broken_frame = Some(record.frame_id);
            out.extend(self.request(KeyframeReason::DecodeError, header.gop_id, now_us));
            return out;
        }
        let keyframe = record.is_keyframe();
        if self.broken == Some(header.gop_id) {
            if !keyframe
                || self
                    .broken_frame
                    .is_some_and(|frame| record.frame_id <= frame)
            {
                return out;
            }
            self.broken = None;
            self.broken_frame = None;
            self.gop = None;
        }
        if self.gop.as_ref().is_none_or(|gop| gop.id != header.gop_id) {
            if !record.is_keyframe() {
                self.broken = Some(header.gop_id);
                self.broken_frame = Some(record.frame_id);
                out.extend(self.request(KeyframeReason::Reset, header.gop_id, now_us));
                return out;
            }
            let limits = header
                .frame_contract
                .map(|contract| contract.limits)
                .or(self.local_limits)
                .unwrap_or(crate::screen::core::frame_budget::FrameLimits::LEGACY);
            if header.frame_contract.is_some_and(|c| c.validate().is_err())
                || limits.check(header.size).is_err()
                || self
                    .queue
                    .set_frame_limits(
                        limits,
                        header.size,
                        header.frame_contract,
                        self.local_limits.is_some(),
                    )
                    .is_err()
            {
                self.broken = Some(header.gop_id);
                self.broken_frame = Some(record.frame_id);
                out.extend(self.request(KeyframeReason::DecodeError, header.gop_id, now_us));
                return out;
            }
            let hybrid = header.frame_contract.is_some_and(|c| c.hybrid_regions);
            let tiles: Option<Box<dyn ScreenDecoder>> = match header.codec {
                CodecKind::H264 => None,
                CodecKind::Tiles if hybrid => None,
                CodecKind::Tiles => {
                    match TilesScreenDecoder::new_with_limits(header.size, limits) {
                        Ok(decoder) => Some(Box::new(
                            crate::screen::core::frame_reference::ReferenceDecoder::new(Box::new(
                                decoder,
                            )),
                        )),
                        Err(_) => {
                            self.broken = Some(header.gop_id);
                            out.extend(self.request(
                                KeyframeReason::DecodeError,
                                header.gop_id,
                                now_us,
                            ));
                            return out;
                        }
                    }
                }
            };
            self.gop = Some(Gop {
                id: header.gop_id,
                tiles,
                hybrid,
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
        if self.gop.as_ref().is_some_and(|gop| gop.hybrid) {
            let parsed = crate::screen::core::frame_protocol::decode(&record.payload, header.size);
            let (stamp, operation) = match parsed {
                Ok(value) if value.0.frame_id == record.frame_id => value,
                _ => {
                    self.broken = Some(header.gop_id);
                    self.broken_frame = Some(record.frame_id);
                    out.extend(self.request(KeyframeReason::DecodeError, header.gop_id, now_us));
                    return out;
                }
            };
            let begins = matches!(
                operation,
                crate::screen::core::frame_protocol::Operation::Begin
            );
            drop(operation);
            if self.new_stream && !begins {
                return out;
            }
            let mut flags = FLAG_HYBRID
                | FLAG_LAST_OF_PICTURE
                | FLAG_GOP_ID
                | FLAG_FRAME_ID
                | (((header.gop_id & 0xFFFF) as u32) << GOP_SHIFT);
            if keyframe {
                flags |= FLAG_KEYFRAME;
            }
            if record.has_more() {
                flags |= FLAG_MORE_PARTS;
            }
            if begins {
                flags |= FLAG_HYBRID_BEGIN;
            }
            if self.new_stream {
                flags |= FLAG_NEW_STREAM;
            }
            let data = pull_identity(header.gop_id, record.frame_id, &record.payload);
            let pushed = self.queue.push(record.capture_ts_us, flags, data, now_us);
            if pushed.queued {
                self.new_stream = false;
            }
            if pushed.stalled {
                self.broken = Some(header.gop_id);
                self.broken_frame = Some(stamp.frame_id);
                out.extend(self.request(KeyframeReason::Reset, header.gop_id, now_us));
            }
            return out;
        }
        let mut flags = if keyframe { FLAG_KEYFRAME } else { 0 };
        let data = match self.gop.as_mut().and_then(|gop| gop.tiles.as_mut()) {
            None if record.has_more() => return out,
            None => {
                let mut flags = flags
                    | FLAG_LAST_OF_PICTURE
                    | FLAG_GOP_ID
                    | FLAG_FRAME_ID
                    | (((header.gop_id & 0xFFFF) as u32) << GOP_SHIFT);
                if self.new_stream {
                    flags |= FLAG_NEW_STREAM;
                }
                let data = pull_identity(header.gop_id, record.frame_id, &record.payload);
                let pushed = self.queue.push(record.capture_ts_us, flags, data, now_us);
                if pushed.queued {
                    self.new_stream = false;
                }
                if pushed.stalled {
                    self.new_stream = true;
                    out.extend(self.request(KeyframeReason::Reset, header.gop_id, now_us));
                }
                return out;
            }
            Some(decoder) => match decoder.decode(record) {
                Ok(true) => {
                    flags |= FLAG_TILES_BGRA;
                    let mut stamped =
                        pull_identity(header.gop_id, record.frame_id, decoder.picture().data());
                    for pixel in stamped[16..].chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                    stamped
                }
                Ok(false) => return out,
                Err(_) => {
                    self.broken = Some(header.gop_id);
                    self.broken_frame = Some(record.frame_id);
                    out.extend(self.request(KeyframeReason::DecodeError, header.gop_id, now_us));
                    return out;
                }
            },
        };
        if keyframe && header.codec == CodecKind::Tiles {
            lock(&self.keyframes).on_keyframe();
        }
        let gop_bits = ((header.gop_id & 0xFFFF) as u32) << GOP_SHIFT;
        flags |= FLAG_LAST_OF_PICTURE | gop_bits;
        if self.new_stream {
            flags |= FLAG_NEW_STREAM;
        }
        flags |= FLAG_GOP_ID | FLAG_FRAME_ID;
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

/// The JNI queue owns immutable identities even after newer stream events arrive.
fn pull_identity(gop_id: u64, frame_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut stamped = Vec::with_capacity(payload.len().saturating_add(16));
    stamped.extend_from_slice(&gop_id.to_le_bytes());
    stamped.extend_from_slice(&frame_id.to_le_bytes());
    stamped.extend_from_slice(payload);
    stamped
}

/// The byte order Android's `ARGB_8888` bitmaps take from a buffer.
pub(crate) fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut out = bgra.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    out
}
