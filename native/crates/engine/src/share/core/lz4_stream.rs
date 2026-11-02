//! LZ4 on the content streams of the Share (`lz4_v1`, E13-S3): which
//! requests may use it, how a sender packs the chunks of one stream, and how
//! a receiver turns a content frame back into raw bytes.
//!
//! Only file content is packed (`Read`, `ReadAt`, `Write`, `WriteNew`,
//! `StageWriteAt`), never a control frame, a path, a name, an error or a
//! listing. Each block stands alone (no state between blocks or files), at
//! most one chunk (`fs::CHUNK`) of raw bytes; the receiver unpacks into a
//! buffer of exactly the announced length and refuses an announced length
//! above one chunk before it unpacks. Both sides must have advertised
//! `lz4_v1` in the authenticated session's capabilities, and the request must
//! ask for it: nothing is packed otherwise. Packed blocks travel as
//! `TAG_DATA_LZ4` frames; a chunk that does not shrink goes as the plain
//! `TAG_DATA` frame it always was.
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent_proto::{free_cores, lz4_switched_off, pack_lz4, unpack};
use crate::transfer::compress_policy::{PROBE_BYTES, PROBE_PERIOD, WINDOW_BLOCKS};

use super::{TAG_DATA, TAG_DATA_LZ4};
use crate::share::transfer_meter as stats;

#[path = "lz4_link.rs"]
mod link;
pub(in crate::share) use link::{link_for, Lz4Link};

/// Kind byte and raw length in front of an LZ4 block (see `TAG_DATA_LZ4`).
const BLOCK_HEAD: usize = 5;

/// Whether this side serves (or uses) LZ4 content for a request that asked
/// for it. The suite can switch it off in a development build.
pub(in crate::share) fn negotiated(requested: bool) -> bool {
    requested && !lz4_switched_off()
}

/// What one stream decides on its own: the hold of the last decision and
/// where its next sample is due. The measurements are its link's.
struct Local {
    decision: bool,
    held: u32,
    next_probe: u64,
    writer: u64,
}

enum Packer {
    /// Decisions from the measurements of the stream's connection (Z-P8).
    Link { link: Lz4Link, local: Local },
    /// One stream's own policy with fixed measurements (tests).
    #[cfg(test)]
    Own(crate::agent_proto::BlockSender),
}

struct State {
    packer: Packer,
    /// Raw bytes of this stream packed or passed so far (the sampling clock).
    offset: u64,
}

/// The packing state of one stream direction. The reader of a file packs on
/// its blocking thread while the stream's async side reports how long the
/// wire made it wait, so both share it.
#[derive(Clone)]
pub(in crate::share) struct Lz4Stream(Arc<Mutex<State>>);

impl Lz4Stream {
    /// A stream with measurements of its own (a link of one stream).
    pub(in crate::share) fn new() -> Self {
        Lz4Link::default().stream()
    }

    fn on_link(link: Lz4Link, writer: u64) -> Self {
        Self(Arc::new(Mutex::new(State {
            packer: Packer::Link {
                link,
                local: Local {
                    decision: false,
                    held: 0,
                    next_probe: 0,
                    writer,
                },
            },
            offset: 0,
        })))
    }

    /// A stream whose policy already has measurements (tests).
    #[cfg(test)]
    pub(in crate::share) fn with_policy(policy: crate::transfer::compress_policy::Policy) -> Self {
        Self(Arc::new(Mutex::new(State {
            packer: Packer::Own(crate::agent_proto::BlockSender::with_policy(policy)),
            offset: 0,
        })))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The LZ4 block for `chunk` when the policy says to compress and the
    /// data shrinks; `None`: send `chunk` as a plain `TAG_DATA` frame.
    pub(in crate::share) fn pack(&self, chunk: &[u8]) -> Option<Vec<u8>> {
        let mut state = self.state();
        let offset = state.offset;
        state.offset = offset.saturating_add(chunk.len() as u64);
        let timer = stats::Timer::start();
        let packed = match &mut state.packer {
            Packer::Link { link, local } => pack_on_link(link, local, chunk, offset),
            #[cfg(test)]
            Packer::Own(sender) => sender.try_pack(chunk, offset),
        };
        timer.stop(stats::Phase::Lz4);
        if packed.is_some() {
            stats::lz4_block();
        }
        packed
    }

    /// `bytes` of raw content went to the wire and the writer waited
    /// `blocked` for it.
    pub(in crate::share) fn record_send(&self, bytes: usize, blocked: Duration, now: Instant) {
        match &mut self.state().packer {
            Packer::Link { link, local } => link.record_send(local.writer, bytes, blocked, now),
            #[cfg(test)]
            Packer::Own(sender) => sender.record_send(bytes, blocked, now),
        }
    }
}

/// The rule of `compress_policy` on a stream of a link: the stream holds a
/// decision for `WINDOW_BLOCKS` blocks and samples the first block of its
/// file and then every `PROBE_PERIOD`; the ratio, the core rate and the
/// wire measurement are the link's, so a new stream (a small file, a part)
/// starts with what its connection already learned.
fn pack_on_link(link: &Lz4Link, local: &mut Local, chunk: &[u8], offset: u64) -> Option<Vec<u8>> {
    let probe = offset >= local.next_probe;
    if probe {
        local.next_probe = (offset / PROBE_PERIOD + 1).saturating_mul(PROBE_PERIOD);
    }
    let compress = if local.held > 0 {
        local.held -= 1;
        local.decision
    } else {
        local.decision = link.input(free_cores()).favours_compression();
        local.held = WINDOW_BLOCKS - 1;
        local.decision
    };
    if compress {
        let started = Instant::now();
        let packed = pack_lz4(chunk);
        let took = started.elapsed();
        let size = packed
            .as_ref()
            .map_or(chunk.len(), |block| block.len().saturating_sub(BLOCK_HEAD));
        link.observe_compression(chunk.len(), size, took);
        return packed;
    }
    if probe {
        // Only a sample is packed to measure; the bytes go out as they are.
        let sample = &chunk[..chunk.len().min(PROBE_BYTES)];
        let started = Instant::now();
        let packed = pack_lz4(sample);
        let took = started.elapsed();
        let size = packed
            .as_ref()
            .map_or(sample.len(), |block| block.len().saturating_sub(BLOCK_HEAD));
        link.observe_compression(sample.len(), size, took);
    }
    None
}

/// The frame to send for `chunk`: the tag and the bytes after packing. A
/// plain chunk keeps its own bytes (no copy).
pub(in crate::share) fn frame_for<'a>(
    stream: Option<&Lz4Stream>,
    chunk: &'a [u8],
) -> (u8, std::borrow::Cow<'a, [u8]>) {
    let frame: (u8, std::borrow::Cow<'a, [u8]>) = match stream.and_then(|stream| stream.pack(chunk))
    {
        Some(block) => (TAG_DATA_LZ4, block.into()),
        None => (TAG_DATA, chunk.into()),
    };
    stats::bytes(chunk.len(), frame.1.len());
    frame
}

/// What a received frame of a content stream is.
pub(in crate::share) enum Content {
    /// File bytes, unpacked.
    Bytes(Vec<u8>),
    /// Not a content frame (a control frame): its payload, untouched.
    Other(Vec<u8>),
}

/// Sorts a received frame: `TAG_DATA` is raw content, `TAG_DATA_LZ4` is
/// unpacked content, anything else is handed back. A `TAG_DATA_LZ4` frame on
/// a stream that did not negotiate LZ4 is `InvalidData`, as is any bad block.
pub(in crate::share) fn classify(tag: u8, payload: Vec<u8>, lz4: bool) -> io::Result<Content> {
    match tag {
        TAG_DATA => {
            stats::bytes(payload.len(), payload.len());
            Ok(Content::Bytes(payload))
        }
        TAG_DATA_LZ4 if lz4 => {
            let timer = stats::Timer::start();
            let raw = unpack(&payload);
            timer.stop(stats::Phase::Lz4);
            if let Ok(raw) = &raw {
                stats::bytes(raw.len(), payload.len());
            }
            raw.map(Content::Bytes)
        }
        TAG_DATA_LZ4 => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Peer sendet LZ4-Daten, die nicht ausgehandelt wurden",
        )),
        _ => Ok(Content::Other(payload)),
    }
}

#[cfg(test)]
#[path = "lz4_stream_tests.rs"]
mod tests;
