//! Portable product-buffer bounds for the selected host codec and snapshots.
use std::io;
use super::capture_plan::{bgra_bytes, CaptureOwner, CapturePhases, CapturePlan, HostPhase};
use super::frame_budget::{FrameContract, FrameLimits};
use super::frame_size_caps::{PacketBudget, RECORD_BUFFER_OVERHEAD};
use super::pixel_convert::FrameSize;
use super::wire::invalid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostContract {Legacy, Versioned(FrameContract)}
impl HostContract {
    pub(crate) fn frame_contract(self) -> Option<FrameContract> {
        match self {Self::Legacy => None, Self::Versioned(contract) => Some(contract)}
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostCodecPlan {
    Tiles,
    OpenH264,
    /// The platform reports the actual cbSize/fallback before allocation.
    MediaFoundation {output_buffer_bytes: u64},
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostInputPixels {BorrowedSource, SeparateImage}
#[derive(Clone, Debug)]
pub(crate) struct HostOutputPlan {
    contract: HostContract,
    size: FrameSize,
    codec: HostCodecPlan,
}
impl HostOutputPlan {
    pub(crate) fn new(contract: HostContract, size: FrameSize, codec: HostCodecPlan) -> io::Result<Self> {
        if let HostContract::Versioned(contract) = contract {contract.validate()?;}
        contract.frame_contract().map_or(FrameLimits::LEGACY, |c| c.limits).check(size)?;
        if let HostCodecPlan::MediaFoundation {output_buffer_bytes} = codec {
            if output_buffer_bytes == 0 || output_buffer_bytes > 64 * 1024 * 1024 {
                return Err(invalid("MF output allocation exceeds existing 64 MiB bound"));
            }
        }
        Ok(Self {contract, size, codec})
    }
    pub(crate) fn contract(&self) -> HostContract {self.contract}
    pub(crate) fn size(&self) -> FrameSize {self.size}
    pub(crate) fn codec(&self) -> HostCodecPlan {self.codec}

    /// Adds local host owners only. A local viewer keeps its independent W.
    /// Native metadata, pending old samples, and resize generations are added
    /// by the prepared source as actual separate owners, before approval.
    pub(crate) fn append_to(&self, source: &mut CapturePlan, input: HostInputPixels) -> io::Result<()> {
        use CaptureOwner as O;
        use HostPhase as P;
        let image = bgra_bytes(self.size)?;
        let tiles = u64::from(self.size.width.div_ceil(64))
            .checked_mul(u64::from(self.size.height.div_ceil(64))).ok_or_else(|| invalid("host tile count overflow"))?;
        let packet = PacketBudget::for_contract(self.contract.frame_contract())?;
        let tiles_output = if self.contract == HostContract::Legacy {
            add(image, mul(tiles, 13)?)?
        } else {packet.encoded_bytes as u64};
        let output = match self.codec {
            HostCodecPlan::Tiles => tiles_output,
            _ => (packet.record_bytes as u64).min(16 * 1024 * 1024),
        };
        let mut plan = CapturePlan::new(source.source().clone(), source.generation());
        plan.bind_contract(self.contract.frame_contract())?;
        let mut put = |owner, bytes, count, phases: &[HostPhase]| -> io::Result<()> {
            plan.add_bytes(owner, bytes, count, CapturePhases::new(phases)?)?;
            Ok(())
        };
        if input == HostInputPixels::SeparateImage {
            put(O::ScaledPixels, image, 1, &[P::LiveEncode, P::RawSnapshot, P::Convert, P::CodecOutput])?;
        }
        if self.codec == HostCodecPlan::Tiles {
            // V: persistent live TileEncoder previous pixels.
            put(O::CodecInput, image, 1, &[P::LiveEncode, P::EncodedTransfer, P::RawSnapshot,
                P::SnapshotEncode, P::Send, P::Convert, P::CodecOutput])?;
        }
        put(O::CodecOutput, output, 1, &[P::LiveEncode, P::EncodedTransfer, P::Send, P::CodecOutput])?;
        // Worker records + serialization + parent payload + decoded records.
        put(O::Transport, output, 3, &[P::EncodedTransfer])?;
        // Worker Raw D/alias + two payloads + decoded Parent image.
        put(O::Transport, image, 2, &[P::RawSnapshot])?;
        put(O::CompositionPixels, image, 1, &[P::RawSnapshot, P::SnapshotEncode])?;
        // Snapshot encoder previous S is distinct from the live previous V.
        put(O::CodecInput, image, 1, &[P::SnapshotEncode])?;
        put(O::CodecOutput, tiles_output, 1, &[P::SnapshotEncode])?;
        if self.contract.frame_contract().is_some_and(|contract| contract.hybrid_regions) {
            // Existing HybridEncoder previous and repeat input are distinct
            // until its consumer is converted to immutable capture aliases.
            put(O::CodecInput, image, 2, &[P::LiveEncode, P::EncodedTransfer,
                P::RawSnapshot, P::SnapshotEncode, P::Send, P::Convert, P::CodecOutput])?;
            // Operation data + serialization/repack buffer before old output
            // is destroyed; each is bounded by the unchanged v2 record R.
            put(O::Transport, packet.record_bytes as u64, 2, &[P::LiveEncode, P::SnapshotEncode])?;
            let per_tile = add(mul(std::mem::size_of::<super::pixel_convert::Rect>() as u64, 2)?,
                add(mul(std::mem::size_of::<super::frame_regions::MoveRect>() as u64, 2)?, 2)?)?;
            put(O::Metadata, mul(tiles, per_tile)?, 1, &[P::LiveEncode, P::SnapshotEncode, P::EncodedTransfer])?;
        }
        let luma = u64::from(self.size.width).checked_mul(u64::from(self.size.height))
            .ok_or_else(|| invalid("host luma overflow"))?;
        let chroma = mul(u64::from(self.size.width.div_ceil(2)), u64::from(self.size.height.div_ceil(2)))?;
        let yuv = add(luma, mul(chroma, 2)?)?;
        match self.codec {
            HostCodecPlan::Tiles => (),
            HostCodecPlan::OpenH264 => put(O::CodecInput, yuv, 1, &[P::LiveEncode, P::CodecOutput])?,
            HostCodecPlan::MediaFoundation {output_buffer_bytes} => {
                put(O::CodecInput, yuv, 2, &[P::Convert])?;
                put(O::CodecInput, yuv, 1, &[P::CodecOutput])?;
                put(O::CodecOutput, output_buffer_bytes, 1, &[P::CodecOutput])?;
                put(O::Transport, output, 1, &[P::CodecOutput])?;
            }
        }
        // Both tile encoders can retain their tile/hash allocations during a
        // snapshot. Include explicit output Vec headers and IPC fixed heads.
        let tile_scratch = add(64 * 64 * 4, lz4_flex::block::get_maximum_output_size(64 * 64 * 4) as u64)?;
        let scratch = mul(tile_scratch, if self.codec == HostCodecPlan::Tiles {2} else {1})?;
        put(O::FilterScratch, scratch, 1, &[P::LiveEncode, P::SnapshotEncode, P::EncodedTransfer, P::RawSnapshot])?;
        let hashes = mul(mul(tiles, std::mem::size_of::<u64>() as u64)?, 2)?;
        let containers = mul(mul(packet.records as u64, std::mem::size_of::<Vec<u8>>() as u64)?, 2)?;
        let metadata = add(add(hashes, containers)?, mul(RECORD_BUFFER_OVERHEAD as u64, 4)?)?;
        put(O::Metadata, metadata, 1, &[P::Capture, P::LiveEncode, P::EncodedTransfer,
            P::RawSnapshot, P::SnapshotEncode, P::Send, P::Convert, P::CodecOutput])?;
        source.add_member(plan)
    }
}
fn add(a: u64, b: u64) -> io::Result<u64> {
    a.checked_add(b).ok_or_else(|| invalid("host ownership sum overflow"))
}
fn mul(a: u64, b: u64) -> io::Result<u64> {
    a.checked_mul(b).ok_or_else(|| invalid("host ownership product overflow"))
}
