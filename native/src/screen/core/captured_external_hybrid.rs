//! Admitted external video AUs paired with admitted raw pixels.
use super::*;
/// Android's surface encoder supplies the admitted AU separately from its
/// admitted raw capture. Repeated repair frames reuse the captured previous.
pub(crate) struct CapturedExternalHybrid(CapturedHybrid);
struct ExternalVideo(FrameSize);
impl ScreenEncoder for ExternalVideo {
    fn name(&self) -> String {"external".into()}
    fn size(&self) -> FrameSize {self.0}
    fn encode(&mut self, _image: &BgraImage, _key: bool, _bps: u64) -> io::Result<EncodedFrame> {
        Err(invalid("external hybrid requires its admitted access unit"))
    }
}
impl CapturedExternalHybrid {
    pub(crate) fn new(request: &StageRequest, grant: super::super::super::capture_admission::CaptureGrant) -> io::Result<Self> {
        let contract = request.frame_contract.filter(|contract| contract.hybrid_regions)
            .ok_or_else(|| invalid("external captured hybrid requires v2 regions"))?;
        contract.validate()?;
        let count = request.size.width.div_ceil(64) as usize * request.size.height.div_ceil(64) as usize;
        let pending_lease = grant.claim(super::super::super::capture_admission::CaptureOwner::Metadata, count as u64)?;
        let mut pending = Vec::new();
        pending.try_reserve_exact(count).map_err(|_| io::Error::other("external hybrid pending allocation failed"))?;
        pending.resize(count,false);
        Ok(Self(CapturedHybrid {video: Box::new(ExternalVideo(request.size)), previous: None, pending,
            _pending_lease: pending_lease, grant, contract, generation: generation(request.encoder_epoch)?,
            last_frame: 0, tiles: false, visible: None}))
    }
    pub(crate) fn needs_refresh(&self) -> bool {self.0.needs_refresh()}
    pub(crate) fn encode_external(&mut self, image: &super::super::super::capture_image::CapturedImage,
        request: &StageRequest, video: super::super::super::host_backend::CapturedEncodedFrame)
        -> io::Result<super::super::super::host_backend::CapturedEncodedFrame> {
        self.0.captured_external(image,request,Some(video))
    }
    pub(crate) fn repeat_external(&mut self, request: &StageRequest)
        -> io::Result<super::super::super::host_backend::CapturedEncodedFrame> {
        let image = self.0.previous.clone().ok_or_else(|| invalid("unchanged capture has no admitted previous pixels"))?;
        self.0.captured_external(&image,request,None)
    }
}
