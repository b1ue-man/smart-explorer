//! Picture storage of the Media Foundation decoder (plan 2026-11-14
//! „Bildkette“ V1, BK9 U1) and its continuation rule (B-36).
//!
//! The decoder hands its output over instead of copying it
//! (`take_yuv_picture`, `take_picture`); the viewer gives the storage of a
//! shown picture back (`recycle_*`), and the next picture is read into it:
//! no allocation per picture while size and format stay. The software
//! stage copies the MFT's locked buffer straight into the two planes (the
//! one necessary copy: the buffer belongs to the MFT), the hardware stage
//! reads its texture back into them (`mf_decoder_d3d.rs`).
use std::io;

use windows::Win32::Media::MediaFoundation::IMFSample;

use super::MfDecoder;
use crate::screen::core::host_backend::{Chroma, YuvPicture};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::BgraImage;

/// B-36: whether a decoder of `codec` decodes a continuation from the
/// references it kept. HEVC (reference picture sets, H.265 8.3.2) and AV1
/// (`ref_frame_idx` into the eight reference slots, AV1 5.9.2) name every
/// reference of a picture explicitly; frames lost after the confirmed one
/// leave the referenced picture in place. H.264 numbers its frames
/// (`frame_num`): the lost frames are a gap that a stream without
/// `gaps_in_frame_num_value_allowed_flag` treats as lost references
/// (H.264 8.2.5.2; OpenH264 reports it as `dsRefLost`), and whether the
/// Microsoft decoder conceals it silently is not proven, so H.264 asks for
/// a key frame, as before.
pub(in super::super) fn keeps_references(codec: CodecKind) -> bool {
    codec != CodecKind::H264
}

impl MfDecoder {
    /// The storage the next picture is read into: an output nobody took,
    /// else a returned or kept one (empty at first).
    pub(super) fn buffers(&mut self) -> (Vec<u8>, Vec<u8>) {
        match self.planes.take().filter(|_| self.yuv) {
            Some(planes) => (planes.y, planes.uv),
            None => std::mem::take(&mut self.texture_planes),
        }
    }

    /// The software stage: the MFT's buffer into the planes, one copy.
    pub(super) fn convert_memory(&mut self, sample: &IMFSample, max: usize) -> io::Result<()> {
        let (mut luma, mut chroma) = self.buffers();
        let (rows, width) = (self.allocated.height as usize, self.size.width as usize);
        let copied = copy_sample(sample, max, |bytes| {
            split_planes(bytes, rows, width, &mut luma, &mut chroma)
        });
        match copied {
            Ok(pitch) => self.keep(luma, chroma, pitch),
            Err(error) => {
                self.texture_planes = (luma, chroma);
                Err(error)
            }
        }
    }

    pub(super) fn hand_over_planes(&mut self) -> Option<YuvPicture> {
        self.planes.take().filter(|_| self.yuv)
    }

    /// The BGRA picture is no reference of the MFT, so it goes; a returned
    /// buffer of the same size takes its place.
    pub(super) fn hand_over_picture(&mut self) -> BgraImage {
        match self.spare_bgra.take() {
            Some(next) if next.size() == self.picture.size() && next.regions().is_none() => {
                std::mem::replace(&mut self.picture, next)
            }
            _ => self.picture.clone(),
        }
    }

    pub(super) fn take_back(&mut self, planes: YuvPicture) {
        if planes.chroma == Chroma::Nv12 && planes.y.capacity() >= self.texture_planes.0.capacity()
        {
            self.texture_planes = (planes.y, planes.uv);
        }
    }

    pub(super) fn take_back_picture(&mut self, image: BgraImage) {
        self.spare_bgra = Some(image);
    }
}

/// Runs `copy` on the bytes of `sample`'s contiguous buffer while it is
/// locked; the same checks as `mf_media::sample_bytes_bounded`, without
/// its owned copy.
fn copy_sample(
    sample: &IMFSample,
    max: usize,
    copy: impl FnOnce(&[u8]) -> io::Result<usize>,
) -> io::Result<usize> {
    // Check before MF coalesces the buffers, which can allocate a second full sample.
    // SAFETY: a length query on an owned output sample.
    if unsafe { sample.GetTotalLength()? } as usize > max {
        return Err(io::Error::other("MF output exceeds admitted sample bytes"));
    }
    // SAFETY: the buffer stays locked only while `copy` reads its bytes;
    // `length` bytes at `data` are readable while locked.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut length = 0u32;
        buffer.Lock(&mut data, None, Some(std::ptr::addr_of_mut!(length)))?;
        let result = if length as usize > max {
            Err(io::Error::other(
                "MF locked output exceeds admitted sample bytes",
            ))
        } else if data.is_null() {
            copy(&[])
        } else {
            copy(std::slice::from_raw_parts(data, length as usize))
        };
        buffer.Unlock()?;
        result
    }
}

/// Splits one NV12 buffer of `rows` padded rows into the luma plane and
/// the chroma rows that follow it; the row pitch follows from the length.
/// Returns the pitch.
pub(super) fn split_planes(
    bytes: &[u8],
    rows: usize,
    width: usize,
    luma: &mut Vec<u8>,
    chroma: &mut Vec<u8>,
) -> io::Result<usize> {
    let pitch = bytes
        .len()
        .checked_mul(2)
        .ok_or_else(|| io::Error::other("NV12 size overflow"))?
        / (rows * 3).max(1);
    if pitch < width || bytes.len() < pitch * rows {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded NV12 picture is smaller than its type says",
        ));
    }
    let (y, uv) = bytes.split_at(pitch * rows);
    luma.clear();
    luma.extend_from_slice(y);
    chroma.clear();
    chroma.extend_from_slice(uv);
    Ok(pitch)
}

#[cfg(test)]
mod tests {
    use super::{keeps_references, split_planes};
    use crate::screen::core::negotiate::CodecKind;

    #[test]
    fn bildkette_task_viewer_mf_planes_into_kept_storage() {
        // 4 × 2 picture, padded to 4 rows of pitch 8: 32 luma, 16 chroma bytes.
        let buffer: Vec<u8> = (0..48).collect();
        let (mut luma, mut chroma) = (Vec::with_capacity(64), Vec::with_capacity(64));
        let storage = (luma.as_ptr(), chroma.as_ptr());
        assert_eq!(
            split_planes(&buffer, 4, 4, &mut luma, &mut chroma).unwrap(),
            8
        );
        assert_eq!(luma, buffer[..32]);
        assert_eq!(chroma, buffer[32..]);
        // One copy each, straight into the kept storage.
        assert_eq!((luma.as_ptr(), chroma.as_ptr()), storage);
        // A buffer shorter than its rows is refused, as before.
        assert!(split_planes(&buffer[..20], 4, 4, &mut luma, &mut chroma).is_err());
        // B-36: HEVC and AV1 continue, H.264 asks for a key frame.
        assert!(keeps_references(CodecKind::Hevc));
        assert!(keeps_references(CodecKind::Av1));
        assert!(!keeps_references(CodecKind::H264));
    }
}
