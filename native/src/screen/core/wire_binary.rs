//! Picture streams (decision C3 „Bildgruppen-Stream“): the host opens one
//! QUIC uni stream per picture group. It starts with a 20-byte head `SCRV`,
//! version `u8`, `gop_id u64`, kind `u8`, codec `u8`, profile `u8`, width
//! `u16`, height `u16`; then frame records of a 21-byte head `len u32,
//! frame_id u64, capture_ts_us u64, flags u8` and `len` payload bytes, all
//! big-endian. A record carries at most 16 MiB (`MAX_FRAME` of the share
//! framing), a record of the tile codec at most 4 MiB. Every stream starts
//! with a key frame; frame ids rise across the session; the records of one
//! tile frame share its id and all but the last carry `FLAG_MORE`.
use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::negotiate::{CodecKind, H264Profile};
use super::pixel_convert::FrameSize;
use super::tiles_codec::MAX_TILES_RECORD_BYTES;
use super::wire::{invalid, BeReader};

pub(crate) const GOP_MAGIC: [u8; 4] = *b"SCRV";
pub(crate) const GOP_VERSION: u8 = 1;
pub(crate) const GOP_HEADER_BYTES: usize = 20;
pub(crate) const RECORD_HEADER_BYTES: usize = 21;
/// Largest record payload: `MAX_FRAME` of `share/core/framing.rs`.
pub(crate) const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// The frame is a key frame (IDR, or a tile frame with every tile).
pub(crate) const FLAG_KEYFRAME: u8 = 0x01;
/// More records of the same tile frame follow.
pub(crate) const FLAG_MORE: u8 = 0x02;
const KNOWN_FLAGS: u8 = FLAG_KEYFRAME | FLAG_MORE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamKind {
    Video,
    /// Tile previews of the group view, acknowledged one by one (C6).
    Preview,
    /// One still image (`SNAPSHOT`, `se screen snapshot`).
    Snapshot,
}

impl StreamKind {
    fn wire_code(self) -> u8 {
        match self {
            Self::Video => 1,
            Self::Preview => 2,
            Self::Snapshot => 3,
        }
    }

    fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Video),
            2 => Some(Self::Preview),
            3 => Some(Self::Snapshot),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GopHeader {
    pub gop_id: u64,
    pub kind: StreamKind,
    pub codec: CodecKind,
    /// Set exactly for H.264.
    pub profile: Option<H264Profile>,
    pub size: FrameSize,
}

impl GopHeader {
    fn check(&self) -> io::Result<()> {
        self.size.check_frame()?;
        match (self.codec, self.profile) {
            (CodecKind::H264, Some(_)) | (CodecKind::Tiles, None) => Ok(()),
            _ => Err(invalid("picture stream profile does not match its codec")),
        }
    }

    pub(crate) fn encode(&self) -> io::Result<[u8; GOP_HEADER_BYTES]> {
        self.check()?;
        let mut out = [0u8; GOP_HEADER_BYTES];
        out[..4].copy_from_slice(&GOP_MAGIC);
        out[4] = GOP_VERSION;
        out[5..13].copy_from_slice(&self.gop_id.to_be_bytes());
        out[13] = self.kind.wire_code();
        out[14] = self.codec.wire_code();
        out[15] = self.profile.map_or(0, H264Profile::wire_code);
        out[16..18].copy_from_slice(&(self.size.width as u16).to_be_bytes());
        out[18..20].copy_from_slice(&(self.size.height as u16).to_be_bytes());
        Ok(out)
    }

    pub(crate) fn decode(bytes: &[u8; GOP_HEADER_BYTES]) -> io::Result<Self> {
        let mut reader = BeReader::new(bytes);
        if reader.bytes(4)? != GOP_MAGIC {
            return Err(invalid("picture stream does not start with SCRV"));
        }
        if reader.u8()? != GOP_VERSION {
            return Err(invalid("unsupported picture stream version"));
        }
        let gop_id = reader.u64()?;
        let kind = StreamKind::from_wire_code(reader.u8()?)
            .ok_or_else(|| invalid("unknown picture stream kind"))?;
        let codec = CodecKind::from_wire_code(reader.u8()?)
            .ok_or_else(|| invalid("unknown picture stream codec"))?;
        let profile = match reader.u8()? {
            0 => None,
            code => Some(
                H264Profile::from_wire_code(code)
                    .ok_or_else(|| invalid("unknown picture stream profile"))?,
            ),
        };
        let size = FrameSize::new(u32::from(reader.u16()?), u32::from(reader.u16()?));
        let header = Self {
            gop_id,
            kind,
            codec,
            profile,
            size,
        };
        header.check()?;
        Ok(header)
    }

    /// Largest record payload of this stream's codec.
    pub(crate) fn record_limit(&self) -> usize {
        match self.codec {
            CodecKind::H264 => MAX_RECORD_BYTES,
            CodecKind::Tiles => MAX_TILES_RECORD_BYTES,
        }
    }
}

/// One frame record as read from a picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameRecord {
    pub frame_id: u64,
    /// Host clock when the frame was captured.
    pub capture_ts_us: u64,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl FrameRecord {
    pub(crate) fn is_keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }

    pub(crate) fn has_more(&self) -> bool {
        self.flags & FLAG_MORE != 0
    }
}

/// Order rules of the records of one picture stream: the first frame is a
/// key frame; frame ids rise; the records of a tile frame keep its id and
/// flags, and only tile frames have more than one record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecordSequence {
    last_frame: Option<u64>,
    open: Option<(u64, u8)>,
}

impl RecordSequence {
    pub(crate) fn accept(&mut self, header: &GopHeader, record: &FrameRecord) -> io::Result<()> {
        if record.flags & !KNOWN_FLAGS != 0 {
            return Err(invalid("frame record has unknown flags"));
        }
        if record.has_more() && header.codec != CodecKind::Tiles {
            return Err(invalid("only tile frames span several records"));
        }
        let base_flags = record.flags & !FLAG_MORE;
        match self.open {
            Some((frame_id, flags)) if frame_id == record.frame_id && flags == base_flags => {}
            Some(_) => return Err(invalid("frame record interrupts an unfinished tile frame")),
            None => {
                let rises = self.last_frame.is_none_or(|last| record.frame_id > last);
                if !rises {
                    return Err(invalid("frame ids of a picture stream must rise"));
                }
                if self.last_frame.is_none() && !record.is_keyframe() {
                    return Err(invalid("a picture stream must start with a key frame"));
                }
                self.last_frame = Some(record.frame_id);
            }
        }
        self.open = record.has_more().then_some((record.frame_id, base_flags));
        Ok(())
    }

    /// Whether the stream ended within a tile frame.
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

pub(crate) async fn write_gop_header<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &GopHeader,
) -> io::Result<()> {
    writer.write_all(&header.encode()?).await?;
    writer.flush().await
}

pub(crate) async fn read_gop_header<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<GopHeader> {
    let mut bytes = [0u8; GOP_HEADER_BYTES];
    reader.read_exact(&mut bytes).await?;
    GopHeader::decode(&bytes)
}

/// Writes one record; a payload above the codec's limit is refused before
/// anything is written.
pub(crate) async fn write_frame_record<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &GopHeader,
    record: &FrameRecord,
) -> io::Result<()> {
    if record.payload.len() > header.record_limit() || record.flags & !KNOWN_FLAGS != 0 {
        return Err(invalid(
            "frame record exceeds its limit or has unknown flags",
        ));
    }
    let mut head = [0u8; RECORD_HEADER_BYTES];
    head[..4].copy_from_slice(&(record.payload.len() as u32).to_be_bytes());
    head[4..12].copy_from_slice(&record.frame_id.to_be_bytes());
    head[12..20].copy_from_slice(&record.capture_ts_us.to_be_bytes());
    head[20] = record.flags;
    writer.write_all(&head).await?;
    writer.write_all(&record.payload).await?;
    writer.flush().await
}

/// Reads the next record; `None` when the stream ended cleanly between two
/// records. The length is checked against the codec's limit before the
/// payload is allocated.
pub(crate) async fn read_frame_record<R: AsyncRead + Unpin>(
    reader: &mut R,
    header: &GopHeader,
) -> io::Result<Option<FrameRecord>> {
    let mut head = [0u8; RECORD_HEADER_BYTES];
    if reader.read(&mut head[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut head[1..]).await?;
    let mut fields = BeReader::new(&head);
    let len = fields.u32()? as usize;
    let frame_id = fields.u64()?;
    let capture_ts_us = fields.u64()?;
    let flags = fields.u8()?;
    if len > header.record_limit() {
        return Err(invalid("frame record exceeds the limit of its codec"));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(FrameRecord {
        frame_id,
        capture_ts_us,
        flags,
        payload,
    }))
}

#[cfg(test)]
#[path = "wire_binary_tests.rs"]
mod tests;
