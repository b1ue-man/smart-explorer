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

use crate::agent_proto::{lz4_switched_off, unpack, BlockSender};

use super::{TAG_DATA, TAG_DATA_LZ4};

/// Whether this side serves (or uses) LZ4 content for a request that asked
/// for it. The suite can switch it off in a development build.
pub(in crate::share) fn negotiated(requested: bool) -> bool {
    requested && !lz4_switched_off()
}

struct State {
    sender: BlockSender,
    /// Raw bytes of this stream packed or passed so far (the sampling clock).
    offset: u64,
}

/// The packing state of one stream direction. The reader of a file packs on
/// its blocking thread while the stream's async side reports how long the
/// wire made it wait, so both share it.
#[derive(Clone)]
pub(in crate::share) struct Lz4Stream(Arc<Mutex<State>>);

impl Lz4Stream {
    pub(in crate::share) fn new() -> Self {
        let mut sender = BlockSender::new();
        sender.start_file();
        Self(Arc::new(Mutex::new(State { sender, offset: 0 })))
    }

    /// A stream whose policy already has measurements (tests).
    #[cfg(test)]
    pub(in crate::share) fn with_policy(policy: crate::transfer::compress_policy::Policy) -> Self {
        Self(Arc::new(Mutex::new(State {
            sender: BlockSender::with_policy(policy),
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
        state.sender.try_pack(chunk, offset)
    }

    /// `bytes` of raw content went to the wire and the writer waited
    /// `blocked` for it.
    pub(in crate::share) fn record_send(&self, bytes: usize, blocked: Duration, now: Instant) {
        self.state().sender.record_send(bytes, blocked, now);
    }
}

/// The frame to send for `chunk`: the tag and the bytes after packing. A
/// plain chunk keeps its own bytes (no copy).
pub(in crate::share) fn frame_for<'a>(
    stream: Option<&Lz4Stream>,
    chunk: &'a [u8],
) -> (u8, std::borrow::Cow<'a, [u8]>) {
    match stream.and_then(|stream| stream.pack(chunk)) {
        Some(block) => (TAG_DATA_LZ4, block.into()),
        None => (TAG_DATA, chunk.into()),
    }
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
        TAG_DATA => Ok(Content::Bytes(payload)),
        TAG_DATA_LZ4 if lz4 => unpack(&payload).map(Content::Bytes),
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
