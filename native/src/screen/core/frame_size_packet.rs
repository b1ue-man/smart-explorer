//! Derived packet and product-buffer bounds for legacy and negotiated frames.
use std::io;

use super::super::wire::invalid;

/// Payload bounds for a single admitted frame, independent of sender queue length.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PacketBudget {
    pub raw_bytes: usize,
    pub records: usize,
    pub encoded_bytes: usize,
    pub record_bytes: usize,
    pub pull_bytes: usize,
}
/// Relay GOP + record + maximum contract extension, plus JNI head/identity.
pub(crate) const RECORD_BUFFER_OVERHEAD: usize =
    super::super::wire_binary::GOP_HEADER_BYTES
        + super::super::wire_binary::RECORD_HEADER_BYTES
        + 1026
        + 32;
impl PacketBudget {
    /// Private receiving bounds do not turn a v1 stream into a v2 contract.
    pub(crate) fn for_receive(
        contract: Option<super::super::frame_budget::FrameContract>,
        local: Option<super::super::frame_budget::FrameLimits>,
    ) -> io::Result<Self> {
        match (contract, local) {
            (Some(contract), _) => Self::for_contract(Some(contract)),
            (None, Some(limits))
                if limits.working_bytes < super::super::frame_budget::MAX_WORKING_BYTES =>
            {
                let (budget, peak) = Self::legacy_memory(limits)?;
                if peak > limits.working_bytes {
                    return Err(invalid(
                        "legacy product buffers exceed admitted working memory",
                    ));
                }
                Ok(budget)
            }
            (None, Some(limits)) => Self::for_limits_mode(limits, true),
            (None, None) => Self::for_contract(None),
        }
    }
    /// v1 cannot negotiate repacking. Its old sender may put the entire tile
    /// picture in one record. Charge that record at every existing queue slot.
    pub(crate) fn legacy_memory(
        limits: super::super::frame_budget::FrameLimits,
    ) -> io::Result<(Self, u64)> {
        limits.validate()?;
        let pixels = limits.max_pixels.min(limits.max_size.pixels());
        let image = pixels
            .checked_mul(4)
            .ok_or_else(|| invalid("legacy image size overflow"))?;
        let columns = u64::from(limits.max_size.width.div_ceil(64));
        let rows = u64::from(limits.max_size.height.div_ceil(64));
        let tiles = (pixels.div_ceil(4096) + columns + rows + 1)
            .min(columns * rows)
            .min(pixels);
        let tiles_bytes = image
            .checked_add(tiles * 13 + 2)
            .ok_or_else(|| invalid("legacy tile packet overflow"))?;
        let record = tiles_bytes
            .max(super::super::tiles_codec::MIN_RECORD_LIMIT as u64)
            .min(super::super::wire_binary::MAX_RECORD_BYTES as u64);
        let encoded = tiles_bytes
            .max(record)
            .checked_add((tiles + 4) * super::super::wire_binary::RECORD_HEADER_BYTES as u64 + 1024)
            .ok_or_else(|| invalid("legacy frame packet overflow"))?;
        // Same product owners as the modern receiver; v1 keeps its larger R.
        let peak = image
            .checked_mul(20)
            .and_then(|n| n.checked_add((record + RECORD_BUFFER_OVERHEAD as u64) * 32))
            .and_then(|n| n.checked_add(encoded - image))
            .ok_or_else(|| invalid("legacy working memory overflow"))?;
        let n = |bytes: u64| {
            usize::try_from(bytes).map_err(|_| invalid("legacy packet exceeds address space"))
        };
        Ok((
            Self {
                raw_bytes: n(image + 8)?,
                records: n(tiles + 4)?,
                encoded_bytes: n(encoded)?,
                record_bytes: n(record)?,
                pull_bytes: n(image.max(record) + 32)?,
            },
            peak.max(image * 24),
        ))
    }
    pub(crate) fn for_limits(limits: super::super::frame_budget::FrameLimits) -> io::Result<Self> {
        // Bare limits describe source/private legacy buffers, not a v2 grant.
        Self::for_limits_mode(limits, true)
    }
    pub(crate) fn for_contract(
        contract: Option<super::super::frame_budget::FrameContract>,
    ) -> io::Result<Self> {
        Self::for_limits_mode(
            contract.map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits),
            contract.is_none(),
        )
    }
    fn for_limits_mode(
        limits: super::super::frame_budget::FrameLimits,
        legacy: bool
    ) -> io::Result<Self> {
        limits.validate()?;
        let images = if legacy {
            super::super::frame_budget::WORKING_IMAGES
        } else {
            super::super::frame_budget::VERSIONED_WORKING_IMAGES
        };
        let pixels = limits
            .max_pixels
            .min(limits.working_bytes / images / 4)
            .min(limits.max_size.pixels());
        let columns = u64::from(limits.max_size.width.div_ceil(64));
        let rows = u64::from(limits.max_size.height.div_ceil(64));
        let tiles = (pixels.div_ceil(4096) + columns + rows + 1)
            .min(columns * rows)
            .min(pixels);
        let to_size = |n: u64| {
            usize::try_from(n).map_err(|_| invalid("frame packet budget exceeds address space"))
        };
        let raw_bytes = to_size(
            pixels
                .checked_mul(4)
                .and_then(|n| n.checked_add(8))
                .ok_or_else(|| invalid("raw frame packet overflow"))?,
        )?;
        let records = to_size(
            tiles
                .checked_add(4)
                .ok_or_else(|| invalid("frame record count overflow"))?,
        )?;
        let tile = lz4_flex::block::get_maximum_output_size(64 * 64 * 4) as u64;
        // One full Video record, two stamps, one move list with at most 2*tiles entries,
        // lossless LZ4 tiles, record lengths and the bounded encoder-name/header fields.
        let image = pixels
            .checked_mul(4)
            .ok_or_else(|| invalid("image budget overflow"))?;
        // Reserve the pixel/metadata ownership first. Thirty-two live record
        // copies plus the queue's video record share the remaining bytes.
        let tile_pixels =
            u64::from(limits.max_size.width.min(64)) * u64::from(limits.max_size.height.min(64));
        let tile_raw = to_size(tile_pixels.min(pixels) * 4)?;
        let floor = lz4_flex::block::get_maximum_output_size(tile_raw)
            .checked_add(41)
            .ok_or_else(|| invalid("tile budget overflow"))?;
        let overhead = image / 255 + tiles * 512 + 1024 + (RECORD_BUFFER_OVERHEAD as u64) * 32;
        let available = limits
            .working_bytes
            .saturating_sub(image * 20)
            .saturating_sub(overhead);
        let record_bytes = if legacy {
            super::super::wire_binary::MAX_RECORD_BYTES
        } else {
            to_size(available / 33)?
                .min(to_size(image)?)
                .max(floor)
                .min(super::super::wire_binary::MAX_RECORD_BYTES)
        };
        let encoded = if legacy {
            tiles
                .checked_mul(tile + 41 + super::super::wire_binary::RECORD_HEADER_BYTES as u64 + 48)
                .and_then(|n| n.checked_add(super::super::wire_binary::MAX_RECORD_BYTES as u64))
                .and_then(|n| {
                    n.checked_add(
                        2 * 25
                            + 29
                            + 4 * super::super::wire_binary::RECORD_HEADER_BYTES as u64
                            + 24
                            + 256
                            + 64,
                    )
                })
                .ok_or_else(|| invalid("encoded frame packet overflow"))?
        } else {
            // LZ4 worst case over the actual pixels, per-tile protocol,
            // move-list and JNI identity overhead, one bounded video AU.
            image
                .checked_add(image / 255)
                .and_then(|n| n.checked_add(tiles * 512))
                .and_then(|n| n.checked_add(record_bytes as u64 + 1024))
                .ok_or_else(|| invalid("encoded frame packet overflow"))?
        };
        if !legacy {
            // Thirteen image slots cover native/decoder images, the sole pull
            // buffer, Full/Preview snapshots and active/next scaling sources.
            // Seven further slots reserve resampling/output ownership. Android
            // leases actual source/target arrays, fixed filter rows and retained
            // outputs against this remainder before allocating. Codec/driver
            // storage is outside this product-buffer contract.
            let peak = image
                .checked_mul(20)
                .and_then(|n| {
                    n.checked_add((record_bytes as u64 + RECORD_BUFFER_OVERHEAD as u64) * 32)
                })
                .and_then(|n| n.checked_add(encoded.saturating_sub(image)));
            if peak.is_none_or(|n| n > limits.working_bytes) {
                return Err(invalid(
                    "screen product buffers exceed negotiated working memory",
                ));
            }
        }
        let pull_bytes = to_size(
            image
                .max(record_bytes as u64)
                .checked_add(32)
                .ok_or_else(|| invalid("pull size overflow"))?,
        )?;
        Ok(Self {
            raw_bytes,
            records,
            encoded_bytes: to_size(encoded)?,
            record_bytes,
            pull_bytes,
        })
    }
}

/// Producer-side rejection keeps the worker alive so the existing Tiles
/// fallback can reuse its source, input ownership and capture connection.
pub(crate) fn validate_encoded_budget(
    contract: Option<super::super::frame_budget::FrameContract>,
    codec: super::super::negotiate::CodecKind,
    records: &[Vec<u8>],
) -> io::Result<()> {
    let Some(contract) = contract else {
        return Ok(());
    };
    contract.validate()?;
    let budget = PacketBudget::for_contract(Some(contract))?;
    let total = records.iter().try_fold(0usize, |n, r| {
        n.checked_add(r.len())
            .and_then(|n| n.checked_add(super::super::wire_binary::RECORD_HEADER_BYTES))
    });
    if records.len() > budget.records
        || total.is_none_or(|n| n > budget.encoded_bytes)
        || ((contract.hybrid_regions || codec == super::super::negotiate::CodecKind::H264)
            && records.iter().any(|r| r.len() > budget.record_bytes))
    {
        return Err(invalid("encoder exceeds negotiated frame-buffer budget"));
    }
    Ok(())
}
