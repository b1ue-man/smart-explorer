//! Admitted NV12 inputs. No temporary unclaimed NV12 Vec is materialized.
use super::mf_buffer_lease;
use super::mf_common::platform;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::{bgra_to_i420_with_limits, YuvMatrix};
use crate::screen::core::wire::invalid;
use std::io;
use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::{IMFMediaBuffer, IMFSample};

pub(super) fn sample_with_buffer(buffer: &IMFMediaBuffer) -> io::Result<IMFSample> {
    let mut raw = std::ptr::null_mut();
    unsafe { (platform()?.create_sample)(&mut raw) }.ok()?;
    let sample = unsafe { IMFSample::from_raw(raw) };
    // AddBuffer holds this wrapper's identity, not its private inner buffer.
    unsafe { sample.AddBuffer(buffer)? };
    Ok(sample)
}

pub(super) fn input_sample(
    grant: &CaptureGrant,
    image: &CapturedImage,
    time: i64,
    duration: i64,
) -> io::Result<IMFSample> {
    grant.check_identity(image.source(), image.generation())?;
    let size = image.size();
    FrameLimits::RESOURCE.check(size)?;
    if size != size.even() {
        return Err(invalid("Media Foundation needs even NV12 geometry"));
    }
    let length = size
        .pixels()
        .checked_add(
            size.chroma()
                .pixels()
                .checked_mul(2)
                .ok_or_else(|| invalid("MF chroma size overflow"))?,
        )
        .ok_or_else(|| invalid("MF NV12 size overflow"))?;
    // The existing converter's three vec![0; length] allocations are exactly
    // Y + U + V, so their maximum is established before this allocator runs.
    let yuv = grant
        .claim(CaptureOwner::CodecInput, length)?
        .allocate(|| {
            bgra_to_i420_with_limits(&image.view(), YuvMatrix::Bt709, FrameLimits::RESOURCE)
        })?;
    let buffer = mf_buffer_lease::create(
        grant,
        CaptureOwner::CodecInput,
        usize::try_from(length).map_err(|_| invalid("MF input exceeds address space"))?,
    )?;
    let mut data = std::ptr::null_mut();
    let mut capacity = 0u32;
    unsafe { buffer.Lock(&mut data, Some(&mut capacity), None)? };
    let filled = (|| -> io::Result<()> {
        if data.is_null() || u64::from(capacity) < length {
            return Err(invalid("MF input buffer contradicts its allocation size"));
        }
        // SAFETY: the locked native buffer has at least length writable bytes.
        let target = unsafe { std::slice::from_raw_parts_mut(data, length as usize) };
        let yuv = yuv.resource();
        target[..yuv.y.len()].copy_from_slice(&yuv.y);
        for (pair, (u, v)) in target[yuv.y.len()..]
            .chunks_exact_mut(2)
            .zip(yuv.u.iter().zip(&yuv.v))
        {
            pair.copy_from_slice(&[*u, *v]);
        }
        Ok(())
    })();
    // Always unlock even when size verification failed.
    let unlocked = unsafe { buffer.Unlock() };
    filled?;
    unlocked?;
    unsafe { buffer.SetCurrentLength(length as u32)? };
    let sample = sample_with_buffer(&buffer)?;
    unsafe {
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
    }
    Ok(sample)
}
