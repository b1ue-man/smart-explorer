//! Portable legacy tile adapters used by decoder and benchmark callers.
use super::*;
/// The lossless tile codec as a `ScreenEncoder`.
pub(crate) struct TilesScreenEncoder {
    encoder: TileEncoder,
}

impl TilesScreenEncoder {
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            encoder: TileEncoder::new(size)?,
        })
    }
}

impl TilesScreenEncoder {
    pub(crate) fn new_with_limits(
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
pub(crate) struct TilesScreenDecoder {
    decoder: TileDecoder,
    decoded: Option<super::super::decoded_frame::DecodedFrame>,
}

impl TilesScreenDecoder {
    pub(crate) fn new(size: FrameSize) -> io::Result<Self> {
        Ok(Self {
            decoder: TileDecoder::new(size)?,
            decoded: None,
        })
    }
}

impl TilesScreenDecoder {
    pub(crate) fn new_with_limits(
        size: FrameSize,
        limits: super::super::frame_budget::FrameLimits,
    ) -> io::Result<Self> {
        Ok(Self {
            decoder: TileDecoder::new_with_limits(size, limits)?,
            decoded: None,
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
}
