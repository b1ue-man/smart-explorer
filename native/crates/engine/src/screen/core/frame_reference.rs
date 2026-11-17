//! Decoder safety boundary shared by v1 and v2. Errors poison references.
use super::decoded_frame::{DecodedFrame, FrameIdentities};
use super::host_backend::{ScreenDecoder, YuvPicture};
use super::negotiate::CodecKind;
use super::pixel_convert::BgraImage;
use super::wire::invalid;
use super::wire_binary::FrameRecord;
use std::io;

pub struct ReferenceDecoder {
    inner: Box<dyn ScreenDecoder>,
    valid: bool,
    primed: bool,
    video_codec: Option<CodecKind>,
    last_frame: Option<u64>,
    pending: Option<DecodedFrame>,
    submitted: FrameIdentities,
    decoded: Option<DecodedFrame>,
    last_output: Option<u64>,
}
impl ReferenceDecoder {
    pub fn new(inner: Box<dyn ScreenDecoder>) -> Self {
        Self {
            inner,
            valid: false,
            primed: false,
            video_codec: None,
            last_frame: None,
            pending: None,
            submitted: FrameIdentities::default(),
            decoded: None,
            last_output: None,
        }
    }
    pub fn new_video(inner: Box<dyn ScreenDecoder>) -> Self {
        Self::new_for_codec(inner, CodecKind::H264)
    }
    pub fn new_for_codec(inner: Box<dyn ScreenDecoder>, codec: CodecKind) -> Self {
        let mut decoder = Self::new(inner);
        decoder.video_codec = codec.is_video().then_some(codec);
        decoder
    }
    fn apply(&mut self, record: &FrameRecord) -> io::Result<bool> {
        let input = DecodedFrame::from_record(record);
        if let Some(codec) = self.video_codec.filter(|_| record.is_keyframe()) {
            if !super::frame_size_caps::video_keyframe(codec, &record.payload) {
                return Err(invalid(match codec {
                    CodecKind::H264 => "H.264 keyframe has no Annex-B IDR",
                    _ => "video keyframe has no random-access picture",
                }));
            }
        }
        if !self.valid && !self.primed && !input.keyframe {
            return Err(invalid("decoder awaits a complete keyframe reference"));
        }
        if self.pending.is_some_and(|pending| pending != input)
            || (self.pending.is_none() && self.last_frame.is_some_and(|id| input.frame_id <= id))
        {
            return Err(invalid("decoder frame reordered or incomplete"));
        }
        self.pending = Some(input);
        if !record.has_more() {
            self.submitted.insert(input.frame_id, input)?;
        }
        let complete = self.inner.decode(record)?;
        if !record.has_more() {
            self.last_frame = Some(input.frame_id);
            self.pending = None;
            if self.video_codec.is_some() {
                self.primed |= input.keyframe;
            }
        }
        if complete {
            let output = self
                .inner
                .decoded_frame()
                .ok_or_else(|| invalid("decoder completed a picture without its identity"))?;
            if output != self.submitted.take(output.frame_id)?
                || self.last_output.is_some_and(|id| output.frame_id <= id)
                || (!self.valid && !self.primed && !output.keyframe)
            {
                return Err(invalid(
                    "decoder output contradicts its submitted reference",
                ));
            }
            self.valid = true;
            self.primed = true;
            self.last_output = Some(output.frame_id);
            self.decoded = Some(output);
            return Ok(true);
        }
        if record.has_more() || self.video_codec.is_some() {
            return Ok(false);
        }
        Err(invalid("decoder did not complete reference picture"))
    }
}
impl ScreenDecoder for ReferenceDecoder {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        self.decoded = None;
        let result = self.apply(record);
        if result.is_err() {
            self.valid = false;
            self.primed = false;
            self.pending = None;
            self.submitted.clear();
        }
        result
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }
    fn picture(&self) -> &BgraImage {
        self.inner.picture()
    }
    fn prefer_yuv(&mut self, on: bool) {
        self.inner.prefer_yuv(on);
    }
    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.valid.then(|| self.inner.yuv_picture()).flatten()
    }
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        if self.valid {
            self.inner.take_yuv_picture()
        } else {
            None
        }
    }
    fn take_picture(&mut self) -> BgraImage {
        self.inner.take_picture()
    }
    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.inner.recycle_yuv(planes);
    }
    fn recycle_picture(&mut self, image: BgraImage) {
        self.inner.recycle_picture(image);
    }
    fn prefer_gpu(&mut self, on: bool) {
        self.inner.prefer_gpu(on);
    }
    fn take_gpu_picture(&mut self) -> Option<super::gpu_picture::GpuPicture> {
        if self.valid {
            self.inner.take_gpu_picture()
        } else {
            None
        }
    }
    fn keeps_references(&self) -> bool {
        self.inner.keeps_references()
    }
}
