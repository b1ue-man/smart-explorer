//! The packets of one picture (`video_transport.rs`): its message
//! (`video_message.rs`) cut into data shards of equal length, the shards
//! spread evenly over blocks of at most `MAX_BLOCK_DATA`, each block with
//! `parity_for` parity shards (`fec.rs`), and every shard sent as one
//! packet with this head, big-endian:
//!
//! | bytes | field |
//! |---|---|
//! | 1 | kind `0xB1` (`VIDEO_KIND`; sound datagrams are `0xA5`) |
//! | 1 | version 1 |
//! | 1 | generation: a picture cut again for a smaller packet size |
//! | 8 | frame id |
//! | 8 | group id |
//! | 1 | block index, 1 block count |
//! | 1 | shard index in its block (data first, then parity) |
//! | 1 | data shards, 1 parity shards of its block |
//! | 2 | shard length |
//! | 4 | message length |
//!
//! then the shard (the last data shard is padded with zeros). Packets go
//! interleaved over the blocks (shard 0 of every block, then shard 1, …), so
//! a burst of losses is spread over the blocks' parity.
use std::io;

use super::super::fec;
use super::super::wire::invalid;

pub const VIDEO_KIND: u8 = 0xB1;
pub const PACKET_VERSION: u8 = 1;
pub const PACKET_HEAD: usize = 30;
/// Data shards of one block at most: 100 + 20 parity shards stay far
/// below the 256 points of the field and keep the parity's cost at about
/// 20 table look-ups per byte of a picture.
pub const MAX_BLOCK_DATA: usize = 100;
/// Blocks of one picture at most (one byte).
pub const MAX_BLOCKS: usize = 255;
/// Parity share in percent (Sunshine's `fec_percentage` default).
pub const PARITY_PERCENT: usize = 20;
/// The smallest shard a carrier may give.
pub const MIN_SHARD: usize = 64;

/// Parity shards of a block with `data` data shards: 20 %, at least one,
/// so one lost packet of a small picture is rebuilt.
pub fn parity_for(data: usize) -> usize {
    (data * PARITY_PERCENT).div_ceil(100).max(1)
}

/// The shard length for packets of at most `max_packet` bytes; `None` when
/// a carrier's packets are too small.
pub fn shard_len_for(max_packet: usize) -> Option<usize> {
    max_packet
        .checked_sub(PACKET_HEAD)
        .filter(|len| *len >= MIN_SHARD)
        .map(|len| len.min(usize::from(u16::MAX)))
}

/// Whether a datagram is a picture packet (its first byte).
pub fn is_video(datagram: &[u8]) -> bool {
    datagram.first() == Some(&VIDEO_KIND)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHead {
    pub generation: u8,
    pub frame_id: u64,
    pub gop_id: u64,
    pub block: u8,
    pub blocks: u8,
    pub shard: u8,
    pub data: u8,
    pub parity: u8,
    pub shard_len: u16,
    pub message_len: u32,
}

impl PacketHead {
    fn write(&self, out: &mut Vec<u8>) {
        out.push(VIDEO_KIND);
        out.push(PACKET_VERSION);
        out.push(self.generation);
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.gop_id.to_be_bytes());
        out.extend_from_slice(&[self.block, self.blocks, self.shard, self.data, self.parity]);
        out.extend_from_slice(&self.shard_len.to_be_bytes());
        out.extend_from_slice(&self.message_len.to_be_bytes());
    }

    /// The head and the shard of a packet; the shard has exactly
    /// `shard_len` bytes.
    pub fn decode(packet: &[u8]) -> io::Result<(Self, &[u8])> {
        if packet.len() < PACKET_HEAD || packet[0] != VIDEO_KIND {
            return Err(invalid("not a picture packet"));
        }
        if packet[1] != PACKET_VERSION {
            return Err(invalid("unsupported picture packet version"));
        }
        let u64_at = |at: usize| {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&packet[at..at + 8]);
            u64::from_be_bytes(bytes)
        };
        let head = Self {
            generation: packet[2],
            frame_id: u64_at(3),
            gop_id: u64_at(11),
            block: packet[19],
            blocks: packet[20],
            shard: packet[21],
            data: packet[22],
            parity: packet[23],
            shard_len: u16::from_be_bytes([packet[24], packet[25]]),
            message_len: u32::from_be_bytes([packet[26], packet[27], packet[28], packet[29]]),
        };
        let shard = &packet[PACKET_HEAD..];
        if shard.len() != usize::from(head.shard_len) {
            return Err(invalid("picture packet shard has the wrong length"));
        }
        Ok((head, shard))
    }
}

/// How a message of `message_len` bytes is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub shard_len: usize,
    pub message_len: usize,
    pub data_shards: usize,
    pub blocks: usize,
}

impl Layout {
    pub fn new(message_len: usize, shard_len: usize) -> io::Result<Self> {
        if shard_len == 0 || shard_len > usize::from(u16::MAX) || message_len == 0 {
            return Err(invalid("picture packet layout needs bytes"));
        }
        if u32::try_from(message_len).is_err() {
            return Err(invalid("picture message too long for its packets"));
        }
        let data_shards = message_len.div_ceil(shard_len);
        let blocks = data_shards.div_ceil(MAX_BLOCK_DATA);
        if blocks > MAX_BLOCKS {
            return Err(invalid("picture needs more than 255 erasure blocks"));
        }
        Ok(Self {
            shard_len,
            message_len,
            data_shards,
            blocks,
        })
    }

    /// The first data shard of block `block` and its count: the shards are
    /// spread evenly, the first blocks take one more.
    pub fn block(&self, block: usize) -> (usize, usize) {
        let base = self.data_shards / self.blocks;
        let extra = self.data_shards % self.blocks;
        let first = block * base + block.min(extra);
        (first, base + usize::from(block < extra))
    }

    /// Whether a packet's head belongs to this layout.
    pub fn matches(&self, head: &PacketHead) -> bool {
        let block = usize::from(head.block);
        if usize::from(head.blocks) != self.blocks || block >= self.blocks {
            return false;
        }
        let (_, data) = self.block(block);
        usize::from(head.data) == data
            && usize::from(head.parity) == parity_for(data)
            && usize::from(head.shard) < data + parity_for(data)
            && usize::from(head.shard_len) == self.shard_len
            && head.message_len as usize == self.message_len
    }
}

/// The packets of `message`, `shard_len` bytes of it per packet.
pub fn packetize(
    message: &[u8],
    shard_len: usize,
    frame_id: u64,
    gop_id: u64,
    generation: u8,
) -> io::Result<Vec<Vec<u8>>> {
    let layout = Layout::new(message.len(), shard_len)?;
    let mut blocks: Vec<Vec<Vec<u8>>> = Vec::with_capacity(layout.blocks);
    for block in 0..layout.blocks {
        let (first, count) = layout.block(block);
        let mut shards: Vec<Vec<u8>> = (first..first + count)
            .map(|index| {
                let start = index * shard_len;
                let end = (start + shard_len).min(message.len());
                let mut shard = message[start..end].to_vec();
                shard.resize(shard_len, 0);
                shard
            })
            .collect();
        let refs: Vec<&[u8]> = shards.iter().map(Vec::as_slice).collect();
        let parity = fec::encode_parity(&refs, parity_for(count))?;
        shards.extend(parity);
        blocks.push(shards);
    }
    let longest = blocks.iter().map(Vec::len).max().unwrap_or(0);
    let mut packets = Vec::with_capacity(blocks.iter().map(Vec::len).sum());
    for shard in 0..longest {
        for (block, shards) in blocks.iter().enumerate() {
            let Some(bytes) = shards.get(shard) else {
                continue;
            };
            let (_, data) = layout.block(block);
            let head = PacketHead {
                generation,
                frame_id,
                gop_id,
                block: block as u8,
                blocks: layout.blocks as u8,
                shard: shard as u8,
                data: data as u8,
                parity: parity_for(data) as u8,
                shard_len: shard_len as u16,
                message_len: message.len() as u32,
            };
            let mut packet = Vec::with_capacity(PACKET_HEAD + shard_len);
            head.write(&mut packet);
            packet.extend_from_slice(bytes);
            packets.push(packet);
        }
    }
    Ok(packets)
}

#[cfg(test)]
#[path = "video_packet_tests.rs"]
mod tests;
