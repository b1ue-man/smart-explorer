//! Exact simultaneous codec and transport phases.
use super::*;
impl HostOutputPlan {
    pub(super) fn append_selected(&self, source: &mut CapturePlan, _input: HostInputPixels) -> io::Result<()> {
        use CaptureOwner as O;
        use HostPhase as P;
        let worker = self.side != HostOutputSide::Parent;
        let parent = self.side != HostOutputSide::Worker;
        let packet = PacketBudget::for_contract(self.contract.frame_contract())?;
        // Grants cover every later Preview/Full/quality size allowed by the
        // immutable contract, including source growth after a small first frame.
        let image = (packet.raw_bytes as u64).checked_sub(8).ok_or_else(||invalid("raw packet bound lacks geometry head"))?;
        let raw = self.raw_bound.map_or(image,|bytes|if self.contract.frame_contract().is_some() {bytes.min(image)} else {bytes});
        let tiles = packet.records as u64;
        let tiles_output = if self.contract == HostContract::Legacy {
            add(image, mul(tiles, 13)?)?
        } else {packet.encoded_bytes as u64};
        let hybrid = self.contract.frame_contract().is_some_and(|contract| contract.hybrid_regions);
        let output = match self.codec {
            _ if hybrid => packet.encoded_bytes as u64,
            HostCodecPlan::Tiles => tiles_output,
            _ => (packet.record_bytes as u64).min(16 * 1024 * 1024),
        };
        let mut plan = CapturePlan::new(source.source().clone(), source.generation());
        plan.bind_contract(self.contract.frame_contract())?;
        let mut put = |owner, bytes, count, phases: &[HostPhase]| -> io::Result<()> {
            plan.add_bytes(owner, bytes, count, CapturePhases::new(phases)?)?;
            Ok(())
        };
        // Future Preview/Full and quality changes may need D even when the
        // initially selected maximum aliases the source. Aliases claim no D.
        if worker {
            put(O::ScaledPixels, image, 1, &[P::LiveEncode, P::RawSnapshot, P::Convert, P::CodecOutput])?;
        }
        if worker && self.codec == HostCodecPlan::Tiles {
            // V: persistent live TileEncoder previous pixels.
            put(O::CodecInput, image, 1, &[P::LiveEncode, P::EncodedTransfer, P::RawSnapshot,
                P::SnapshotEncode, P::Send, P::Convert, P::CodecOutput])?;
        }
        if worker {put(O::CodecOutput, output, 1, &[P::LiveEncode, P::EncodedTransfer, P::Send, P::CodecOutput])?;}
        if parent {put(O::CodecOutput, output, 1, &[P::EncodedTransfer, P::Send])?;}
        // Worker records + serialization + parent payload + decoded records.
        put(O::Transport, output, worker as u64 + parent as u64, &[P::EncodedTransfer])?;
        // Worker Raw D/alias + two payloads + decoded Parent image.
        put(O::Transport, raw, worker as u64 + parent as u64, &[P::RawSnapshot])?;
        let wire_heads=(packet.records as u64).checked_mul(4).and_then(|bytes|bytes.checked_add(RECORD_BUFFER_OVERHEAD as u64))
            .ok_or_else(||invalid("worker transport header overflow"))?;
        put(O::Transport,wire_heads,worker as u64+parent as u64,&[P::EncodedTransfer,P::RawSnapshot])?;
        if parent {put(O::CompositionPixels, raw, 1, &[P::RawSnapshot, P::SnapshotEncode, P::Convert])?;
            put(O::ScaledPixels, image, 1, &[P::Convert])?;}
        // Snapshot encoder previous S is distinct from the live previous V.
        if parent {put(O::CodecInput, image, 1, &[P::SnapshotEncode])?;
            put(O::CodecOutput, tiles_output, 1, &[P::SnapshotEncode])?;}
        if worker && self.contract.frame_contract().is_some_and(|contract| contract.hybrid_regions) {
            // Existing HybridEncoder previous and repeat input are distinct
            // until its consumer is converted to immutable capture aliases.
            put(O::CodecInput, image, 2, &[P::LiveEncode, P::EncodedTransfer,
                P::RawSnapshot, P::SnapshotEncode, P::Send, P::Convert, P::CodecOutput])?;
            // The native AU and its final hybrid record coexist during wrapping.
            put(O::CodecOutput, packet.record_bytes as u64, 1,
                &[P::LiveEncode, P::SnapshotEncode])?;
            put(O::Metadata,
                (super::super::visibility::MAX_VISIBLE_RECTS * std::mem::size_of::<super::super::visibility::VisibleRect>()) as u64,
                1, &[P::LiveEncode, P::SnapshotEncode, P::EncodedTransfer])?;
            // Operation data + serialization/repack buffer before old output
            // is destroyed; each is bounded by the unchanged v2 record R.
            put(O::Transport, packet.record_bytes as u64, 2, &[P::LiveEncode, P::SnapshotEncode])?;
            let per_tile = add(mul(std::mem::size_of::<super::super::pixel_convert::Rect>() as u64, 2)?,
                add(mul(std::mem::size_of::<super::super::frame_regions::MoveRect>() as u64, 2)?, 2)?)?;
            put(O::Metadata, mul(tiles, per_tile)?, 1, &[P::LiveEncode, P::SnapshotEncode, P::EncodedTransfer])?;
        }
        let limits=self.contract.frame_contract().map_or(FrameLimits::LEGACY,|contract|contract.limits);
        let pixels=image/4;let mut yuv=0;
        for width in 1..=u64::from(limits.max_size.width).min(pixels) {
            let height=u64::from(limits.max_size.height).min(pixels/width);
            yuv=yuv.max(add(mul(width,height)?,mul(mul(width.div_ceil(2),height.div_ceil(2))?,2)?)?);
        }
        match self.codec {
            _ if !worker => (),
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
        let tile_scratch = add(64 * 64 * 4, (lz4_flex::block::get_maximum_output_size(64 * 64 * 4) + 4) as u64)?;
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

pub(super) fn add(a: u64, b: u64) -> io::Result<u64> {
    a.checked_add(b).ok_or_else(|| invalid("host ownership sum overflow"))
}
pub(super) fn mul(a: u64, b: u64) -> io::Result<u64> {
    a.checked_mul(b).ok_or_else(|| invalid("host ownership product overflow"))
}

