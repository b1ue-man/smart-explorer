//! Prior claims for the product-owned I420 planes and Annex B copy.
use std::io;
use openh264::encoder::EncodedBitStream;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_grant::AdmittedResource;
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::pixel_convert::{bgra_to_i420_with_limits, FrameSize, I420Image, YuvMatrix};
use crate::screen::core::wire::invalid;
use crate::screen::core::wire_binary::MAX_RECORD_BYTES;

pub(super) fn capture_limit(grant: &CaptureGrant, frame: &CapturedImage,
    request: &StageRequest, size: FrameSize) -> io::Result<usize> {
    grant.check_identity(frame.source(), frame.generation())?;
    if frame.size() != size || request.size != size {
        return Err(invalid("capture geometry differs from the OpenH264 encoder"));
    }
    if grant.plan().contract_binding() != Some(request.frame_contract) {
        return Err(invalid("OpenH264 request differs from its admitted contract"));
    }
    let packet = crate::screen::core::frame_size_caps::PacketBudget::for_contract(request.frame_contract)?;
    Ok(packet.record_bytes.min(MAX_RECORD_BYTES))
}

pub(super) fn admitted_i420(grant: &CaptureGrant, frame: &CapturedImage)
    -> io::Result<AdmittedResource<I420Image>> {
    let size = frame.size();
    FrameLimits::RESOURCE.check(size)?;
    let bytes = size.pixels().checked_add(size.chroma().pixels().checked_mul(2)
        .ok_or_else(|| invalid("OpenH264 chroma size overflow"))?)
        .ok_or_else(|| invalid("OpenH264 I420 size overflow"))?;
    // pixel_convert allocates exactly three zeroed planes at these lengths;
    // no unbounded library allocator or geometric Vec growth is involved.
    grant.claim(CaptureOwner::CodecInput, bytes)?.allocate(|| {
        bgra_to_i420_with_limits(&frame.view(), YuvMatrix::Bt709, FrameLimits::RESOURCE)
    })
}

/// Validate the native lengths before Layer::nal_unit's unchecked pointer
/// arithmetic, then copy into a fixed allocation rather than to_vec growth.
pub(super) fn access_unit_len(stream: &EncodedBitStream<'_>, maximum: usize) -> io::Result<usize> {
    let info = stream.raw_info();
    let count = usize::try_from(info.iLayerNum).map_err(|_| invalid("negative OpenH264 layer count"))?;
    if count > info.sLayerInfo.len() {return Err(invalid("OpenH264 layer count exceeds native storage"));}
    let mut total = 0usize;
    for layer in &info.sLayerInfo[..count] {
        let count = usize::try_from(layer.iNalCount).map_err(|_| invalid("negative OpenH264 NAL count"))?;
        if count != 0 && (layer.pNalLengthInByte.is_null() || layer.pBsBuf.is_null()) {
            return Err(invalid("OpenH264 returned missing NAL storage"));
        }
        for index in 0..count {
            // SAFETY: OpenH264 owns count native NAL lengths until the next
            // encoder call, excluded by EncodedBitStream's mutable borrow.
            let length = unsafe {*layer.pNalLengthInByte.add(index)};
            let length = usize::try_from(length).map_err(|_| invalid("negative OpenH264 NAL length"))?;
            total = total.checked_add(length).filter(|n| *n <= maximum)
                .ok_or_else(|| invalid("OpenH264 access unit exceeds its record budget"))?;
        }
    }
    Ok(total)
}

pub(super) fn copy_access_unit(grant: &CaptureGrant, stream: &EncodedBitStream<'_>, maximum: usize)
    -> io::Result<CapturedRecords> {
    let length = access_unit_len(stream, maximum)?;
    if length == 0 {return CapturedRecords::empty(grant);}
    let mut bytes = AdmittedBytes::buffer(grant, CaptureOwner::CodecOutput, length as u64)?;
    for index in 0..stream.num_layers() {
        let layer = stream.layer(index).ok_or_else(|| invalid("OpenH264 layer disappeared"))?;
        for index in 0..layer.nal_count() {
            let nal = layer.nal_unit(index).ok_or_else(|| invalid("OpenH264 NAL disappeared"))?;
            bytes.extend_from_slice(nal)?;
        }
    }
    CapturedRecords::one(bytes)
}
