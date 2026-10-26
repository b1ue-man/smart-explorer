//! Product-owned MF output buffers and a single fixed, admitted AU copy.
use super::mf_buffer_lease;
use super::mf_capture_media::sample_with_buffer;
use super::mf_media::Output;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::host_backend::CapturedEncodedFrame;
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;
use std::{io, mem::ManuallyDrop};
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFTransform, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE,
};

pub(super) fn capture_limit(
    grant: &CaptureGrant,
    image: &CapturedImage,
    request: &StageRequest,
    size: FrameSize,
) -> io::Result<usize> {
    grant.check_identity(image.source(), image.generation())?;
    if image.size() != size || request.size != size {
        return Err(invalid("capture geometry differs from the MF encoder"));
    }
    if grant.plan().contract_binding() != Some(request.frame_contract) {
        return Err(invalid("MF request differs from its admitted contract"));
    }
    let packet =
        crate::screen::core::frame_size_caps::PacketBudget::for_contract(request.frame_contract)?;
    Ok(packet
        .record_bytes
        .min(crate::screen::core::wire_binary::MAX_RECORD_BYTES))
}

pub(super) fn process_output(
    transform: &IMFTransform,
    grant: &CaptureGrant,
    fallback: usize,
) -> io::Result<Output> {
    let info = unsafe { transform.GetOutputStreamInfo(0)? };
    let length = fallback.max(info.cbSize as usize);
    if length > 64 * 1024 * 1024 {
        return Err(invalid("MF output exceeds its resource limit"));
    }
    let provides =
        (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
    let sample = if info.dwFlags & provides != 0 {
        None
    } else {
        let buffer = mf_buffer_lease::create(grant, CaptureOwner::CodecOutput, length)?;
        Some(sample_with_buffer(&buffer)?)
    };
    let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: 0,
        pSample: ManuallyDrop::new(sample),
        dwStatus: 0,
        pEvents: ManuallyDrop::new(None),
    }];
    let mut status = 0u32;
    let result = unsafe { transform.ProcessOutput(0, &mut buffers, &mut status) };
    let (sample, events) = unsafe {
        (
            ManuallyDrop::take(&mut buffers[0].pSample),
            ManuallyDrop::take(&mut buffers[0].pEvents),
        )
    };
    drop(events);
    match result {
        Ok(()) => sample
            .map(Output::Sample)
            .ok_or_else(|| invalid("MF gave no output sample")),
        Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(Output::NeedInput),
        Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => Ok(Output::StreamChange),
        Err(error) => Err(io::Error::other(error)),
    }
}

/// Copies existing buffers individually. ConvertToContiguousBuffer would
/// create an additional native product copy before a matching claim exists.
pub(super) fn append_sample(bytes: &mut AdmittedBytes, sample: &IMFSample) -> io::Result<()> {
    let length = unsafe { sample.GetTotalLength()? } as usize;
    if bytes
        .len()
        .checked_add(length)
        .is_none_or(|n| n as u64 > bytes.capacity())
    {
        return Err(invalid("MF access unit exceeds its admitted record budget"));
    }
    let count = unsafe { sample.GetBufferCount()? };
    let start = bytes.len();
    for index in 0..count {
        let buffer = unsafe { sample.GetBufferByIndex(index)? };
        let mut data = std::ptr::null_mut();
        let (mut length, mut capacity) = (0u32, 0u32);
        unsafe { buffer.Lock(&mut data, Some(&mut capacity), Some(&mut length))? };
        let copied = (|| -> io::Result<()> {
            if length > capacity || (length != 0 && data.is_null()) {
                return Err(invalid("MF output buffer contradicts its locked size"));
            }
            if length != 0 {
                let source = unsafe { std::slice::from_raw_parts(data, length as usize) };
                bytes.extend_from_slice(source)?;
            }
            Ok(())
        })();
        let unlocked = unsafe { buffer.Unlock() };
        copied?;
        unlocked?;
    }
    if bytes.len() - start != length {
        return Err(invalid("MF sample length changed while copying"));
    }
    Ok(())
}

pub(super) fn finish(
    bytes: AdmittedBytes,
    size: FrameSize,
    keyframe: bool,
    wanted: bool,
) -> io::Result<CapturedEncodedFrame> {
    if bytes.is_empty() {
        return Err(invalid("Media Foundation gave no picture for the frame"));
    }
    if wanted && !keyframe {
        return Err(invalid("Media Foundation gave no key frame on request"));
    }
    crate::screen::core::frame_size_caps::check_avc_dimensions(
        bytes.as_slice(),
        size,
        crate::screen::core::frame_budget::FrameLimits::RESOURCE,
    )?;
    Ok(CapturedEncodedFrame {
        records: CapturedRecords::one(bytes)?,
        keyframe,
    })
}
