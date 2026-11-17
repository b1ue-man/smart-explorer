//! Product-owned MF output buffers and a single fixed, admitted AU copy.
//!
//! Key-frame rule D8 (plan 2026-11-17 „Windows-Bild“ P3): a picture counts
//! as a key frame only when the MFT's `MFSampleExtension_CleanPoint` and the
//! stream (`video_keyframe`) agree; a contradiction is an encoder failure
//! (the next encoder of the rank follows). A key frame without its
//! parameter sets gets the last ones the encoder gave, else those of its
//! output type (`MF_MT_MPEG_SEQUENCE_HEADER`), as `vendor_encode.rs` does.
use super::mf_buffer_lease;
use super::mf_capture_media::sample_with_buffer;
use super::mf_media::Output;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::AdmittedBytes;
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::frame_size_caps::{
    complete_unit, completed, marked_keyframe, stream_parameter_sets,
};
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire::invalid;
use std::{io, mem::ManuallyDrop};
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFTransform, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
    MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_MPEG_SEQUENCE_HEADER,
};

/// Largest sequence header of an output type that is read (as the
/// parameter budget of `check_avc_dimensions`).
const MAX_SEQUENCE_HEADER: u32 = 65_536;

/// The parameter sets the encoder's current output type names
/// (`MF_MT_MPEG_SEQUENCE_HEADER`, D8), when it names any.
pub(super) fn sequence_header(transform: &IMFTransform) -> Option<Vec<u8>> {
    // SAFETY: reads of the transform's current output type and its blob
    // into a buffer of the size the type reports.
    unsafe {
        let media = transform.GetOutputCurrentType(0).ok()?;
        let size = media.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()?;
        if size == 0 || size > MAX_SEQUENCE_HEADER {
            return None;
        }
        let mut bytes = vec![0u8; size as usize];
        media
            .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut bytes, None)
            .ok()?;
        Some(bytes)
    }
}

/// D8 for one output sample of `codec`: a sample of one buffer is read in
/// place, and when it starts the access unit a key frame in it without
/// parameter sets gets `kept` or `header` in front. A sample of several
/// buffers is copied as it is (`finish` refuses a key frame of it without
/// parameter sets).
pub(super) fn append_unit_sample(
    bytes: &mut AdmittedBytes,
    sample: &IMFSample,
    codec: CodecKind,
    kept: &mut Option<Vec<u8>>,
    header: Option<&[u8]>,
) -> io::Result<()> {
    if unsafe { sample.GetBufferCount()? } != 1 || codec == CodecKind::Tiles {
        return append_sample(bytes, sample);
    }
    let buffer = unsafe { sample.GetBufferByIndex(0)? };
    let mut data = std::ptr::null_mut();
    let (mut length, mut capacity) = (0u32, 0u32);
    unsafe { buffer.Lock(&mut data, Some(&mut capacity), Some(&mut length))? };
    let copied = (|| -> io::Result<()> {
        if length > capacity || (length != 0 && data.is_null()) {
            return Err(invalid("MF output buffer contradicts its locked size"));
        }
        if length == 0 {
            return Ok(());
        }
        // SAFETY: the locked buffer holds `length` bytes until `Unlock`.
        let unit = unsafe { std::slice::from_raw_parts(data, length as usize) };
        let (prefix, at) = if bytes.is_empty() {
            let completion = complete_unit(codec, unit, kept, header)?;
            (completion.prefix, completion.at.min(unit.len()))
        } else {
            (Vec::new(), 0)
        };
        let total = bytes
            .len()
            .saturating_add(prefix.len())
            .saturating_add(unit.len());
        if total as u64 > bytes.capacity() {
            return Err(invalid("MF access unit exceeds its admitted record budget"));
        }
        bytes.extend_from_slice(&unit[..at])?;
        bytes.extend_from_slice(&prefix)?;
        bytes.extend_from_slice(&unit[at..])
    })();
    let unlocked = unsafe { buffer.Unlock() };
    copied?;
    unlocked?;
    Ok(())
}

/// D8 for the paths without admission: the access unit of one call with
/// its mark checked against the stream and its parameter sets completed.
pub(super) fn checked_unit(
    codec: CodecKind,
    bytes: Vec<u8>,
    marked: bool,
    wanted: bool,
    kept: &mut Option<Vec<u8>>,
    header: Option<&[u8]>,
) -> io::Result<EncodedFrame> {
    if bytes.is_empty() {
        return Err(invalid("Media Foundation gave no picture for the frame"));
    }
    let keyframe = marked_keyframe(codec, &bytes, marked)?;
    if wanted && !keyframe {
        return Err(invalid("Media Foundation gave no key frame on request"));
    }
    let completion = complete_unit(codec, &bytes, kept, header)?;
    let unit = if completion.prefix.is_empty() {
        bytes
    } else {
        completed(&bytes, &completion)
    };
    Ok(EncodedFrame {
        records: vec![unit],
        keyframe,
    })
}

pub(super) fn capture_limit(
    grant: &CaptureGrant,
    picture: &Picture,
    request: &StageRequest,
    size: FrameSize,
) -> io::Result<usize> {
    grant.check_identity(picture.source(), picture.generation())?;
    if picture.size() != size || request.size != size {
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

/// The checked access unit of `codec` (H.264, and HEVC/AV1 by merge W1:
/// their parameter sets are checked as the decoder will check them).
pub(super) fn finish(
    bytes: AdmittedBytes,
    codec: CodecKind,
    size: FrameSize,
    marked: bool,
    wanted: bool,
) -> io::Result<CapturedEncodedFrame> {
    if bytes.is_empty() {
        return Err(invalid("Media Foundation gave no picture for the frame"));
    }
    // D8: the mark counts only when the stream agrees.
    let keyframe = marked_keyframe(codec, bytes.as_slice(), marked)?;
    if wanted && !keyframe {
        return Err(invalid("Media Foundation gave no key frame on request"));
    }
    if keyframe && stream_parameter_sets(codec, bytes.as_slice()).is_none() {
        return Err(invalid(
            "Media Foundation gave a key frame without parameter sets",
        ));
    }
    crate::screen::core::frame_size_caps::check_video_dimensions(
        codec,
        bytes.as_slice(),
        size,
        crate::screen::core::frame_budget::FrameLimits::RESOURCE,
    )?;
    Ok(CapturedEncodedFrame {
        records: CapturedRecords::one(bytes)?,
        keyframe,
    })
}
