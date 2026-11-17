//! The decoder side of the screen backend (`ScreenDecoder`): one decoder
//! per codec, its picture as BGRA, planes or a GPU surface. Split from
//! `host_backend.rs` (merge of plan 2026-11-17 P3, P8 and the bit rate).
use super::*;

pub trait ScreenDecoder: Send {
    fn name(&self) -> String;
    /// Applies one record; `true` when an actual picture is available. A video
    /// decoder may return an older submitted picture; use `decoded_frame` for
    /// its identity, never the record most recently passed to this method.
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool>;
    /// Identity of the picture produced by the last `decode` that returned `true`.
    /// `None` is unproven output and must never receive a presentation receipt.
    fn decoded_frame(&self) -> Option<super::super::decoded_frame::DecodedFrame> {
        None
    }
    /// The last completed picture.
    fn picture(&self) -> &BgraImage;
    /// Asks to keep the decoded planes (`yuv_picture`) and to skip the
    /// conversion to BGRA; `picture()` then stays the last converted one
    /// (black if there never was one) and is not to be read. Only the H.264
    /// decoders honour it; the tile decoder always makes BGRA.
    fn prefer_yuv(&mut self, _on: bool) {}
    /// The planes of the last completed picture after `prefer_yuv(true)`.
    fn yuv_picture(&self) -> Option<YuvPicture> {
        None
    }
    /// Hands the planes of the last completed picture over instead of
    /// copying them (plan „Bildkette“ BK9 U1, V1); afterwards
    /// `yuv_picture` is unspecified until the next picture. The default
    /// copies (`yuv_picture`).
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.yuv_picture()
    }
    /// Hands the last completed BGRA picture over; the default copies it.
    /// A decoder whose picture is its own reference (tiles) copies into a
    /// returned buffer.
    fn take_picture(&mut self) -> BgraImage {
        self.picture().clone()
    }
    /// Gives the storage of a shown picture back, so the next one needs no
    /// new allocation while size and format stay; the default drops it.
    fn recycle_yuv(&mut self, _planes: YuvPicture) {}
    fn recycle_picture(&mut self, _image: BgraImage) {}
    /// P8 (2026-11-17): a presenter on the decoder's device takes pictures as
    /// GPU surfaces; only FFmpeg's D3D11VA stage honours it.
    fn prefer_gpu(&mut self, _on: bool) {}
    /// The last picture as a GPU surface after `prefer_gpu(true)`, once.
    fn take_gpu_picture(&mut self) -> Option<super::super::gpu_picture::GpuPicture> {
        None
    }
    /// B-36: whether this decoder may go on over a continuation
    /// (`FLAG_CONTINUE`) and decode it correctly from its kept references;
    /// `false` makes the viewer ask for a key frame instead (`gop_continue`).
    fn keeps_references(&self) -> bool {
        false
    }
}
