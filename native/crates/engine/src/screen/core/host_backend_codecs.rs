//! Portable legacy tile adapters and shared codec data structures.
use super::*;

/// How the chroma planes of a decoded picture are stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chroma {
    /// One plane with interleaved U and V samples (`YuvPicture::uv`).
    Nv12,
    /// Separate planes of the same row pitch (`YuvPicture::uv_stride`);
    /// `YuvPicture::uv` stays empty.
    I420 { u: Vec<u8>, v: Vec<u8> },
}

/// The planes of a decoded picture in limited range, as the decoder made
/// them (SCR-M7, decision C4: the viewer converts them to RGB on the GPU).
/// Rows are `y_stride` and `uv_stride` bytes apart; the picture's size is
/// the one of its picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YuvPicture {
    pub y: Vec<u8>,
    pub y_stride: usize,
    pub uv: Vec<u8>,
    pub uv_stride: usize,
    pub chroma: Chroma,
    pub matrix: YuvMatrix,
}

/// Why a source cannot be opened; the key is the reason of `no_display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRefusal {
    pub reason: String,
}

impl SourceRefusal {
    pub fn new(reason: &str) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// The lossless tile codec as a `ScreenEncoder`.
pub struct TilesScreenEncoder {
    encoder: TileEncoder,
}

impl TilesScreenEncoder {
    pub fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            encoder: TileEncoder::new(size)?,
        })
    }
}

impl TilesScreenEncoder {
    pub fn new_with_limits(
        size: FrameSize,
        limits: super::super::frame_budget::FrameLimits,
    ) -> io::Result<Self> {
        Ok(Self {
            encoder: TileEncoder::new_with_limits(size, limits)?,
        })
    }
}

impl ScreenEncoder for TilesScreenEncoder {
    fn name(&self) -> String {
        "tiles".into()
    }

    fn size(&self) -> FrameSize {
        self.encoder.size()
    }
    fn encode_frame(
        &mut self,
        frame: &BgraImage,
        request: &super::super::host_stage::StageRequest,
    ) -> io::Result<EncodedFrame> {
        let records = self.encoder.encode_visible(
            &frame.view(),
            request.keyframe,
            MAX_TILES_RECORD_BYTES,
            request.visible_rects.as_deref(),
        )?;
        Ok(EncodedFrame {
            records,
            keyframe: request.keyframe,
        })
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        _target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        let records = self
            .encoder
            .encode(&frame.view(), keyframe, MAX_TILES_RECORD_BYTES)?;
        Ok(EncodedFrame { records, keyframe })
    }
}

/// The lossless tile codec as a `ScreenDecoder`.
pub struct TilesScreenDecoder {
    decoder: TileDecoder,
    decoded: Option<super::super::decoded_frame::DecodedFrame>,
    /// A shown picture given back (`recycle_picture`); the next hand-over
    /// copies into it instead of allocating.
    spare: Option<BgraImage>,
}

impl TilesScreenDecoder {
    pub fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            decoder: TileDecoder::new(size)?,
            decoded: None,
            spare: None,
        })
    }
}

impl TilesScreenDecoder {
    pub fn new_with_limits(
        size: FrameSize,
        limits: super::super::frame_budget::FrameLimits,
    ) -> io::Result<Self> {
        Ok(Self {
            decoder: TileDecoder::new_with_limits(size, limits)?,
            decoded: None,
            spare: None,
        })
    }
}

impl ScreenDecoder for TilesScreenDecoder {
    fn name(&self) -> String {
        "tiles".into()
    }

    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        let complete = self.decoder.apply_frame_record(record)?;
        if complete {
            self.decoded = Some(super::super::decoded_frame::DecodedFrame::from_record(
                record,
            ));
        }
        Ok(complete)
    }

    fn decoded_frame(&self) -> Option<super::super::decoded_frame::DecodedFrame> {
        self.decoded
    }

    fn picture(&self) -> &BgraImage {
        self.decoder.image()
    }

    /// The picture is the reference of the next tile frame, so it stays;
    /// the hand-over is the one necessary copy, into a returned buffer of
    /// the same size when there is one.
    fn take_picture(&mut self) -> BgraImage {
        let image = self.decoder.image();
        match self.spare.take() {
            // Regions cannot be cleared, so a spare with regions serves
            // only a picture that has regions of its own (same as a clone).
            Some(mut spare)
                if spare.size() == image.size()
                    && (spare.regions().is_none() || image.regions().is_some()) =>
            {
                spare.data_mut().copy_from_slice(image.data());
                match image.regions() {
                    Some(regions) => match spare.set_regions(regions.clone()) {
                        Ok(()) => spare,
                        Err(_) => image.clone(),
                    },
                    None => spare,
                }
            }
            _ => image.clone(),
        }
    }

    fn recycle_picture(&mut self, image: BgraImage) {
        if image.size() == self.decoder.image().size() {
            self.spare = Some(image);
        }
    }
}
