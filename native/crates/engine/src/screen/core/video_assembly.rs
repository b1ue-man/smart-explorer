//! The viewer puts the packets of each picture together
//! (`video_packet.rs`) and rebuilds lost packets from parity (`fec.rs`).
//! Times are the viewer's µs; nothing here reads a clock.
//!
//! Rules (`video_transport.rs`):
//! - A picture is complete when every block has as many shards as data
//!   shards; then its message goes on at once.
//! - A picture still incomplete when a newer one completes is given up, and
//!   so is one without a packet for `give_up_us` (`max(2 frame intervals,
//!   50 ms)`, the interval measured between the first packets of
//!   successive pictures) together with every older one; given-up and
//!   delivered pictures are finished, and later packets of them are late.
//! - Memory is bounded: at most `MAX_ASSEMBLING` pictures, each at most
//!   the largest message the viewer accepted (`video_message::Admission`),
//!   all of them together at most twice that plus their parity; a new
//!   picture beyond the bound gives the oldest one up.
//! - The host's last picture before a still screen (`VIDEO_TAIL`) that is
//!   not finished `give_up_us` after the tail came is given up as well,
//!   also when not one packet of it arrived (`abandon_through`).
use std::collections::BTreeMap;
use std::io;

use super::super::fec;
use super::super::wire::invalid;
use super::video_packet::{Layout, PacketHead, PARITY_PERCENT};

/// Pictures in assembly at most.
pub const MAX_ASSEMBLING: usize = 8;
/// The shortest wait for the rest of a picture.
pub const MIN_GIVE_UP_US: u64 = 50_000;
/// The frame interval assumed before two pictures were seen (60 Hz).
const START_INTERVAL_US: u64 = 16_667;
/// Bounds of one measured interval: a still screen sends nothing for long.
const MAX_INTERVAL_US: u64 = 250_000;
const MIN_INTERVAL_US: u64 = 1_000;

/// What the assembler hands on, in rising frame ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assembled {
    /// A whole message; `recovered` data shards were rebuilt.
    Complete {
        gop_id: u64,
        frame_id: u64,
        message: Vec<u8>,
        recovered: usize,
    },
    /// A picture given up; `missing` data shards were neither received nor
    /// rebuilt.
    GaveUp {
        gop_id: u64,
        frame_id: u64,
        missing: usize,
    },
}

struct Block {
    data: usize,
    shards: Vec<Option<Vec<u8>>>,
    received: usize,
    done: bool,
    recovered: usize,
}

struct Picture {
    gop_id: u64,
    generation: u8,
    layout: Layout,
    blocks: Vec<Block>,
    last_us: u64,
    bytes: usize,
}

impl Picture {
    fn new(head: &PacketHead, layout: Layout, now_us: u64) -> Self {
        let blocks = (0..layout.blocks)
            .map(|block| {
                let (_, data) = layout.block(block);
                let total = data + super::video_packet::parity_for(data);
                Block {
                    data,
                    shards: vec![None; total],
                    received: 0,
                    done: false,
                    recovered: 0,
                }
            })
            .collect();
        Self {
            gop_id: head.gop_id,
            generation: head.generation,
            layout,
            blocks,
            last_us: now_us,
            bytes: 0,
        }
    }

    fn complete(&self) -> bool {
        self.blocks.iter().all(|block| block.done)
    }

    /// Data shards neither received nor rebuilt.
    fn missing(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| {
                block.shards[..block.data]
                    .iter()
                    .filter(|s| s.is_none())
                    .count()
            })
            .sum()
    }

    fn message(self) -> Vec<u8> {
        let mut message = Vec::with_capacity(self.layout.data_shards * self.layout.shard_len);
        for block in self.blocks {
            for shard in block.shards.into_iter().take(block.data).flatten() {
                message.extend_from_slice(&shard);
            }
        }
        message.truncate(self.layout.message_len);
        message
    }
}

/// Counters for `VIDEO_REPORT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssemblyCounters {
    pub late_packets: u64,
    pub refused_packets: u64,
    pub lost_packets: u64,
    pub recovered_packets: u64,
    pub recovered_pictures: u64,
    pub lost_pictures: u64,
}

pub struct Assembler {
    max_message: usize,
    pictures: BTreeMap<u64, Picture>,
    /// Frame ids at or below are finished.
    floor: u64,
    interval_us: u64,
    last_first_us: Option<u64>,
    pub counters: AssemblyCounters,
}

impl Assembler {
    /// `max_message`: the largest message the viewer accepts.
    pub fn new(max_message: usize) -> Self {
        Self {
            max_message,
            pictures: BTreeMap::new(),
            floor: 0,
            interval_us: START_INTERVAL_US,
            last_first_us: None,
            counters: AssemblyCounters::default(),
        }
    }

    /// How long a picture may go without a packet.
    pub fn give_up_us(&self) -> u64 {
        (2 * self.interval_us).max(MIN_GIVE_UP_US)
    }

    /// When the next picture is given up for silence, if any is assembled.
    pub fn deadline_us(&self) -> Option<u64> {
        let give_up = self.give_up_us();
        self.pictures
            .values()
            .map(|picture| picture.last_us.saturating_add(give_up))
            .min()
    }

    /// The bytes all pictures may hold together: two messages and parity.
    fn byte_bound(&self) -> usize {
        let one = self.max_message + self.max_message * PARITY_PERCENT / 100;
        one.saturating_mul(2)
    }

    pub fn assembling(&self) -> usize {
        self.pictures.len()
    }

    /// One packet at `now_us`.
    pub fn push(&mut self, packet: &[u8], now_us: u64) -> Vec<Assembled> {
        let mut out = Vec::new();
        let Ok((head, shard)) = PacketHead::decode(packet) else {
            self.counters.refused_packets += 1;
            return out;
        };
        if head.frame_id <= self.floor {
            self.counters.late_packets += 1;
            return out;
        }
        let Ok(layout) = Layout::new(head.message_len as usize, usize::from(head.shard_len)) else {
            self.counters.refused_packets += 1;
            return out;
        };
        if layout.message_len > self.max_message || !layout.matches(&head) {
            self.counters.refused_packets += 1;
            return out;
        }
        let known = self
            .pictures
            .get(&head.frame_id)
            .map(|picture| (picture.generation, picture.layout));
        match known {
            // A newer cut of the picture replaces the older one.
            Some((generation, _)) if generation < head.generation => {
                self.pictures.remove(&head.frame_id);
            }
            Some((generation, cut)) if generation > head.generation || cut != layout => {
                self.counters.refused_packets += 1;
                return out;
            }
            _ => {}
        }
        if !self.pictures.contains_key(&head.frame_id) {
            self.note_first(now_us);
            while self.pictures.len() >= MAX_ASSEMBLING {
                self.give_up_oldest(&mut out);
            }
            // Older than the pictures given up for room: late as well.
            if head.frame_id <= self.floor {
                self.counters.late_packets += 1;
                return out;
            }
            self.pictures
                .insert(head.frame_id, Picture::new(&head, layout, now_us));
        }
        // A block that cannot be rebuilt although it has enough shards is
        // no picture: it is given up.
        if self.store(&head, shard, now_us).is_err() {
            self.give_up_through(head.frame_id, &mut out);
            return out;
        }
        while self.held_bytes() > self.byte_bound() && self.pictures.len() > 1 {
            self.give_up_oldest(&mut out);
        }
        if self
            .pictures
            .get(&head.frame_id)
            .is_some_and(Picture::complete)
        {
            self.give_up_below(head.frame_id, &mut out);
            if let Some(picture) = self.pictures.remove(&head.frame_id) {
                self.floor = head.frame_id;
                let recovered: usize = picture.blocks.iter().map(|b| b.recovered).sum();
                self.counters.lost_packets += recovered as u64;
                self.counters.recovered_packets += recovered as u64;
                self.counters.recovered_pictures += u64::from(recovered > 0);
                out.push(Assembled::Complete {
                    gop_id: picture.gop_id,
                    frame_id: head.frame_id,
                    message: picture.message(),
                    recovered,
                });
            }
        }
        out
    }

    /// Whether `frame_id` is finished: handed on or given up.
    pub fn finished(&self, frame_id: u64) -> bool {
        frame_id <= self.floor
    }

    /// The host's last picture before a still screen (`VIDEO_TAIL`) did not
    /// complete in time: it is given up with every older one; `true` in the
    /// second place when packets of it were in assembly (else no packet of
    /// it came, and it counts as lost here). Its late packets stay late.
    pub fn abandon_through(&mut self, frame_id: u64) -> (Vec<Assembled>, bool) {
        let mut out = Vec::new();
        let present = self.pictures.contains_key(&frame_id);
        self.give_up_through(frame_id, &mut out);
        if !present && !self.finished(frame_id) {
            self.counters.lost_pictures += 1;
        }
        self.floor = self.floor.max(frame_id);
        (out, present)
    }

    /// The task waited `by_us` for the decoder and read no packet meanwhile:
    /// that time is no silence of the line (2026-11-25: pictures whose
    /// packets sat unread in the socket buffer were given up, 13 key frames
    /// and 337 skipped pictures in ten seconds without one lost packet).
    pub fn postpone(&mut self, by_us: u64) {
        for picture in self.pictures.values_mut() {
            picture.last_us = picture.last_us.saturating_add(by_us);
        }
    }

    /// Gives up the pictures silent at `now_us`, each with every older one.
    pub fn expire(&mut self, now_us: u64) -> Vec<Assembled> {
        let mut out = Vec::new();
        let give_up = self.give_up_us();
        let silent = self
            .pictures
            .iter()
            .filter(|(_, picture)| now_us.saturating_sub(picture.last_us) >= give_up)
            .map(|(frame, _)| *frame)
            .max();
        if let Some(frame) = silent {
            self.give_up_through(frame, &mut out);
        }
        out
    }

    fn note_first(&mut self, now_us: u64) {
        if let Some(last) = self.last_first_us {
            let gap = now_us
                .saturating_sub(last)
                .clamp(MIN_INTERVAL_US, MAX_INTERVAL_US);
            self.interval_us = (self.interval_us * 7 + gap) / 8;
        }
        self.last_first_us = Some(now_us);
    }

    fn held_bytes(&self) -> usize {
        self.pictures.values().map(|picture| picture.bytes).sum()
    }

    fn store(&mut self, head: &PacketHead, shard: &[u8], now_us: u64) -> io::Result<()> {
        let picture = self
            .pictures
            .get_mut(&head.frame_id)
            .ok_or_else(|| invalid("picture not in assembly"))?;
        picture.last_us = now_us;
        let block = &mut picture.blocks[usize::from(head.block)];
        let slot = &mut block.shards[usize::from(head.shard)];
        if block.done || slot.is_some() {
            return Ok(());
        }
        *slot = Some(shard.to_vec());
        block.received += 1;
        picture.bytes += shard.len();
        if block.received >= block.data {
            block.recovered = fec::reconstruct(block.data, &mut block.shards)?;
            block.done = true;
        }
        Ok(())
    }

    fn give_up_oldest(&mut self, out: &mut Vec<Assembled>) {
        if let Some(frame) = self.pictures.keys().next().copied() {
            self.give_up_through(frame, out);
        }
    }

    /// Gives up every picture below `frame_id`.
    fn give_up_below(&mut self, frame_id: u64, out: &mut Vec<Assembled>) {
        let older: Vec<u64> = self.pictures.range(..frame_id).map(|(f, _)| *f).collect();
        for frame in older {
            self.give_up(frame, out);
        }
    }

    /// Gives up `frame_id` and every older picture.
    fn give_up_through(&mut self, frame_id: u64, out: &mut Vec<Assembled>) {
        self.give_up_below(frame_id, out);
        self.give_up(frame_id, out);
    }

    fn give_up(&mut self, frame_id: u64, out: &mut Vec<Assembled>) {
        if let Some(picture) = self.pictures.remove(&frame_id) {
            let missing = picture.missing();
            self.counters.lost_packets += missing as u64;
            self.counters.lost_pictures += 1;
            self.floor = self.floor.max(frame_id);
            out.push(Assembled::GaveUp {
                gop_id: picture.gop_id,
                frame_id,
                missing,
            });
        }
    }
}

#[cfg(test)]
#[path = "video_assembly_tests.rs"]
mod tests;
