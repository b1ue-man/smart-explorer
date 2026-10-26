//! Productive fixed-buffer tile encoding.
use super::phases::{add, mul};
use super::*;
/// The productive tile encoder owns admitted fixed scratch and previous pixels.
pub(crate) struct AdmittedTilesEncoder {
    size: FrameSize,
    grant: super::super::capture_admission::CaptureGrant,
    previous: super::super::capture_image::CaptureImageBuilder,
    hashes: super::super::capture_bytes::AdmittedBytes,
    tile: super::super::capture_bytes::AdmittedBytes,
    compressed: super::super::capture_bytes::AdmittedBytes,
    primed: bool,
}
impl AdmittedTilesEncoder {
    pub(crate) fn new(
        size: FrameSize,
        limits: FrameLimits,
        grant: super::super::capture_admission::CaptureGrant,
    ) -> io::Result<Self> {
        use super::super::capture_bytes::AdmittedBytes;
        let tiles = mul(
            u64::from(size.width.div_ceil(64)),
            u64::from(size.height.div_ceil(64)),
        )?;
        let previous = super::super::capture_image::CaptureImageBuilder::new(
            size,
            limits,
            &grant,
            CaptureOwner::CodecInput,
        )?;
        let hashes = AdmittedBytes::new(&grant, CaptureOwner::Metadata, mul(tiles, 8)?)?;
        let tile = AdmittedBytes::new(&grant, CaptureOwner::FilterScratch, 64 * 64 * 4)?;
        let compressed = AdmittedBytes::new(
            &grant,
            CaptureOwner::FilterScratch,
            lz4_flex::block::get_maximum_output_size(64 * 64 * 4) as u64,
        )?;
        Ok(Self {
            size,
            grant,
            previous,
            hashes,
            tile,
            compressed,
            primed: false,
        })
    }
    fn captured(
        &mut self,
        image: &super::super::capture_image::CapturedImage,
        request: &super::super::host_stage::StageRequest,
    ) -> io::Result<super::super::host_backend::CapturedEncodedFrame> {
        use super::super::capture_records::{CaptureRecordBuilder, CapturedRecords};
        use super::super::tiles_codec::{solid_colour, tile_hash, tile_rect};
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
        let mut records: Option<CaptureRecordBuilder> = None;
        let mut count = 0u16;
        let view = image.view();
        let columns = self.size.width.div_ceil(64);
        for row in 0..self.size.height.div_ceil(64) {
            for column in 0..columns {
                let rect = tile_rect(column, row, self.size);
                if !full
                    && !super::super::frame_regions::visible(rect, request.visible_rects.as_deref())
                {
                    continue;
                }
                let hash = tile_hash(&view, rect);
                let slot = (u64::from(row) * u64::from(columns) + u64::from(column)) as usize * 8;
                let old = u64::from_le_bytes(
                    self.hashes.as_slice()[slot..slot + 8]
                        .try_into()
                        .map_err(|_| invalid("tile hash slot is incomplete"))?,
                );
                let start = rect.x as usize * 4;
                let end = start + rect.width as usize * 4;
                let exact = old == hash
                    && (rect.y..rect.y + rect.height).all(|y| {
                        self.previous.view().row(y)[start..end]
                            .chunks_exact(4)
                            .zip(view.row(y)[start..end].chunks_exact(4))
                            .all(|(a, b)| a[..3] == b[..3])
                    });
                if !full && exact {
                    continue;
                }
                let length = rect.width as usize * rect.height as usize * 4;
                let tile = &mut self.tile.as_mut_slice()[..length];
                for y in 0..rect.height {
                    let offset = y as usize * rect.width as usize * 4;
                    tile[offset..offset + rect.width as usize * 4]
                        .copy_from_slice(&view.row(rect.y + y)[start..end]);
                }
                for pixel in tile.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
                let solid = solid_colour(tile);
                let compressed = if solid.is_none() {
                    lz4_flex::block::compress_into(tile, self.compressed.as_mut_slice())
                        .map_err(|_| invalid("admitted tile compression failed"))?
                } else {
                    0
                };
                let (encoding, bytes): (u8, &[u8]) = if let Some(colour) = solid.as_ref() {
                    (0, colour)
                } else if compressed < tile.len() {
                    (2, &self.compressed.as_slice()[..compressed])
                } else {
                    (1, tile)
                };
                if records.is_none() {
                    records = Some(CaptureRecordBuilder::new(
                        &self.grant,
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
                self.hashes.as_mut_slice()[slot..slot + 8].copy_from_slice(&hash.to_le_bytes());
                for y in rect.y..rect.y + rect.height {
                    let target = self
                        .previous
                        .row_mut(y)
                        .ok_or_else(|| invalid("tile previous row missing"))?;
                    target[start..end].copy_from_slice(&view.row(y)[start..end]);
                    for pixel in target[start..end].chunks_exact_mut(4) {
                        pixel[3] = 255;
                    }
                }
            }
        }
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
