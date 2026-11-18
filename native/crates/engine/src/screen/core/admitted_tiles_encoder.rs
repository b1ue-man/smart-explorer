//! Productive fixed-buffer tile encoding. Since P9 (plan 2026-11-17) the
//! encoder compares only the tiles the capture's dirty and move
//! rectangles touched (`tiles_dirty::DirtyTiles`): a tile no rectangle of
//! any picture since it was last looked at touched did not change. A
//! picture the host process captured but dropped before encoding took its
//! rectangles with it (`note_dropped_picture`), and a picture without
//! rectangles, a key frame and a failed frame mark every tile. The
//! comparison and the compression run on every core (`tiles_parallel.rs`),
//! the records keep the tile order of the sequential encoder.
use std::sync::atomic::{AtomicU64, Ordering};

use super::phases::{add, mul};
use super::*;
use crate::screen::core::tiles_dirty::DirtyTiles;
use parallel::{Look, TileWork, Workers};

#[path = "tiles_parallel.rs"]
mod parallel;

/// Pictures captured and dropped before encoding in this process so far.
static DROPPED_PICTURES: AtomicU64 = AtomicU64::new(0);

/// A captured picture was dropped before encoding (the host process's
/// capture stage replaced or discarded it): its dirty rectangles are lost,
/// so the next tile frame looks at every tile. The count is per process; a
/// host process serves one session, and a drop of another session only
/// costs one full look.
pub fn note_dropped_picture() {
    DROPPED_PICTURES.fetch_add(1, Ordering::AcqRel);
}

fn dropped_pictures() -> u64 {
    DROPPED_PICTURES.load(Ordering::Acquire)
}

/// rayon's pool where it has more than one thread, else this thread.
fn workers() -> Workers {
    if rayon::current_num_threads() > 1 {
        Workers::Pool
    } else {
        Workers::Caller
    }
}
/// The productive tile encoder owns admitted fixed scratch and previous pixels.
pub struct AdmittedTilesEncoder {
    size: FrameSize,
    grant: super::super::capture_admission::CaptureGrant,
    previous: super::super::capture_image::CaptureImageBuilder,
    /// `TILE_WAVE` slots of `TILE_SLOT_BYTES` (one where the grant admits
    /// no more: the same output, compressed one tile at a time).
    slots: super::super::capture_bytes::AdmittedBytes,
    primed: bool,
    /// The tiles that may have changed since they were last looked at.
    dirty: DirtyTiles,
    /// `dropped_pictures()` at the last frame.
    drops_seen: u64,
    /// Change flags and wave of the parallel phases (`tiles_parallel.rs`).
    work: TileWork,
}
impl AdmittedTilesEncoder {
    pub fn new(
        size: FrameSize,
        limits: FrameLimits,
        grant: super::super::capture_admission::CaptureGrant,
    ) -> io::Result<Self> {
        use super::super::capture_bytes::AdmittedBytes;
        use super::super::tiles_codec::{TILE_SLOT_BYTES, TILE_WAVE};
        let previous = super::super::capture_image::CaptureImageBuilder::new(
            size,
            limits,
            &grant,
            CaptureOwner::CodecInput,
        )?;
        let slots = AdmittedBytes::new(
            &grant,
            CaptureOwner::FilterScratch,
            mul(TILE_WAVE as u64, TILE_SLOT_BYTES as u64)?,
        )
        .or_else(|_| {
            AdmittedBytes::new(&grant, CaptureOwner::FilterScratch, TILE_SLOT_BYTES as u64)
        })?;
        Ok(Self {
            size,
            grant,
            previous,
            slots,
            primed: false,
            dirty: DirtyTiles::new(size),
            drops_seen: dropped_pictures(),
            work: TileWork::new(size),
        })
    }
    fn captured(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &super::super::host_stage::StageRequest,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        use super::super::capture_records::{CaptureRecordBuilder, CapturedRecords};
        self.grant
            .check_identity(image.source(), image.generation())?;
        if image.size() != self.size {
            return Err(invalid("tile capture geometry changed"));
        }
        let packet = PacketBudget::for_contract(request.frame_contract)?;
        let tiles = mul(
            u64::from(self.size.width.div_ceil(64)),
            u64::from(self.size.height.div_ceil(64)),
        )?;
        let capacity = if request.frame_contract.is_none() {
            add(bgra_bytes(self.size)?, mul(tiles, 13)?)?
        } else {
            packet.encoded_bytes as u64
        };
        let limit = packet
            .record_bytes
            .min(super::super::tiles_codec::MAX_TILES_RECORD_BYTES);
        let full = request.keyframe || !self.primed;
        let drops = dropped_pictures();
        if full || drops != self.drops_seen {
            self.dirty.mark_all();
        } else {
            self.dirty.note(image.size(), image.image().regions());
        }
        self.drops_seen = drops;
        // Phase 1 on every core: compare, take over changed tiles (P9).
        let look = Look {
            view: image.view(),
            full,
            visible: request.visible_rects.as_deref(),
        };
        self.work.look(
            self.previous.pixels_mut(),
            &look,
            &mut self.dirty,
            workers(),
        )?;
        // Phase 2 on every core, records in tile order on this thread.
        let grant = &self.grant;
        let mut records: Option<CaptureRecordBuilder> = None;
        let mut count = 0u16;
        self.work.encode(
            self.previous.view(),
            self.slots.as_mut_slice(),
            workers(),
            |rect, encoding, bytes| {
                if records.is_none() {
                    records = Some(CaptureRecordBuilder::new(
                        grant,
                        capacity,
                        packet.records as u64,
                    )?);
                }
                let output = records
                    .as_mut()
                    .ok_or_else(|| invalid("tile record builder missing"))?;
                if output.remaining() < bytes.len() + 11 || count == u16::MAX {
                    if output.last_len() != 0 {
                        output.overwrite(0, &count.to_be_bytes())?;
                    }
                    output.start(limit as u64, &[0, 0])?;
                    count = 0;
                }
                let mut header = [0u8; 11];
                header[..2].copy_from_slice(&(rect.x as u16).to_be_bytes());
                header[2..4].copy_from_slice(&(rect.y as u16).to_be_bytes());
                header[4] = rect.width as u8;
                header[5] = rect.height as u8;
                header[6] = encoding;
                header[7..].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
                output.append(&header)?;
                output.append(bytes)?;
                count += 1;
                Ok(())
            },
        )?;
        self.primed = true;
        let records = if let Some(mut records) = records {
            records.overwrite(0, &count.to_be_bytes())?;
            records.freeze()
        } else {
            CapturedRecords::empty(&self.grant)?
        };
        Ok(super::super::host_backend::CapturedEncodedFrame {
            records,
            keyframe: full,
        })
    }
}
impl super::super::host_backend::ScreenEncoder for AdmittedTilesEncoder {
    fn name(&self) -> String {
        "tiles".into()
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn encode_captured(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &super::super::host_stage::StageRequest,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        let result = self.captured(image, request);
        if result.is_err() {
            self.primed = false;
            self.dirty.mark_all();
        }
        result
    }
    fn encode(
        &mut self,
        _image: &super::super::pixel_convert::BgraImage,
        _keyframe: bool,
        _bps: u64,
    ) -> io::Result<super::super::host_backend::EncodedFrame> {
        Err(invalid("admitted tile encoder requires captured input"))
    }
}
