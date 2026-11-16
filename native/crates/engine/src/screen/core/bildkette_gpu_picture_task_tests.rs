//! Task suite „Bildkette“ BK3b: a picture on the GPU reaches a consumer of
//! CPU pixels only through its readback, scaling it does no pixel work, and
//! an encoder that takes GPU pictures gets them as they are. Portable: a
//! test surface stands for the Windows ring slot and the Linux DMA-BUF. Not
//! registered yet (`docs/tests/register.tsv`): the manager adds the rows.
use super::*;
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind};
use crate::screen::core::pixel_convert::{BgraImage, MAX_FRAME_PIXELS};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

const NATIVE: FrameSize = FrameSize::new(64, 36);
const COLOUR: u8 = 0x42;

/// A surface that counts its readbacks.
#[derive(Default)]
struct TestSurface {
    gone: AtomicBool,
    reads: AtomicUsize,
}

impl GpuSurface for TestSurface {
    fn native(&self) -> FrameSize {
        NATIVE
    }
    fn slot(&self) -> u64 {
        1
    }
    fn held(&self) -> bool {
        !self.gone.load(Ordering::SeqCst)
    }
    fn read_back(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<CapturedImage> {
        if !self.held() {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut image = CaptureImageBuilder::new(size, limits, grant, owner)?;
        image.pixels_mut().fill(COLOUR);
        Ok(image.freeze())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An encoder of CPU pixels: records the pictures it got.
#[derive(Default)]
struct CpuEncoder {
    got: Mutex<Vec<(FrameSize, u8)>>,
}

impl ScreenEncoder for CpuEncoder {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        _request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let first = frame.image().data()[0];
        self.got.lock().unwrap().push((frame.size(), first));
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::empty(&frame.grant())?,
            keyframe: false,
        })
    }
    fn name(&self) -> String {
        "cpu-test".into()
    }
    fn size(&self) -> FrameSize {
        NATIVE
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("captured input only"))
    }
}

/// An encoder that takes GPU pictures as they are.
#[derive(Default)]
struct GpuEncoder {
    on_gpu: Vec<bool>,
}

impl ScreenEncoder for GpuEncoder {
    fn encode_picture(
        &mut self,
        picture: &Picture,
        _request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        self.on_gpu.push(picture.on_gpu());
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::empty(&picture.grant())?,
            keyframe: false,
        })
    }
    fn name(&self) -> String {
        "gpu-test".into()
    }
    fn size(&self) -> FrameSize {
        NATIVE
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("captured input only"))
    }
}

fn request(size: FrameSize) -> StageRequest {
    StageRequest {
        frame_id: 1,
        frame_contract: None,
        visible_rects: None,
        keyframe: false,
        target_bps: 1_000_000,
        size,
        choice: CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::MAX,
            max_pixels: MAX_FRAME_PIXELS,
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 1,
        fps: 60,
    }
}

#[test]
fn bildkette_task_chain_gpu_picture_reaches_no_cpu_consumer_unread() {
    let grant = crate::screen::core::capture_test_support::grant(NATIVE, None);
    let surface = Arc::new(TestSurface::default());
    let gpu =
        GpuPicture::new(surface.clone(), NATIVE, FrameLimits::LEGACY, &grant).expect("GPU picture");
    let picture = Picture::Gpu(gpu);
    // The stream scales it to the encoder's size: no pixel work.
    let small = FrameSize::new(32, 18);
    let scaled = picture
        .scaled(small, FrameLimits::LEGACY, CaptureOwner::ScaledPixels)
        .expect("scaled");
    assert!(scaled.on_gpu());
    assert_eq!(scaled.size(), small);
    assert_eq!(
        surface.reads.load(Ordering::SeqCst),
        0,
        "scaling reads nothing"
    );
    // An encoder of CPU pixels gets the picture only read back, at its size.
    let mut cpu = CpuEncoder::default();
    cpu.encode_picture(&scaled, &request(small))
        .expect("encoded");
    assert_eq!(surface.reads.load(Ordering::SeqCst), 1);
    assert_eq!(*cpu.got.lock().unwrap(), vec![(small, COLOUR)]);
    // An encoder that takes GPU pictures gets it as it is.
    let mut gpu_encoder = GpuEncoder::default();
    gpu_encoder
        .encode_picture(&scaled, &request(small))
        .expect("encoded");
    assert_eq!(gpu_encoder.on_gpu, vec![true]);
    assert_eq!(
        surface.reads.load(Ordering::SeqCst),
        1,
        "no readback for it"
    );
    // The CPU interface of a source (still image, every other caller).
    let frame = SourcePicture::Frame(picture.clone())
        .into_source_frame()
        .expect("read back");
    match frame {
        SourceFrame::Frame(image) => assert_eq!(image.image().data()[0], COLOUR),
        _ => panic!("a picture was expected"),
    }
    assert_eq!(surface.reads.load(Ordering::SeqCst), 2);
    // A picture the GPU no longer holds never becomes pixels: the frame is
    // skipped (`WouldBlock`) and the encoder is not called.
    surface.gone.store(true, Ordering::SeqCst);
    let error = cpu
        .encode_picture(&scaled, &request(small))
        .expect_err("skipped");
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(cpu.got.lock().unwrap().len(), 1);
    // A CPU picture passes through without any readback and without a copy.
    let pixels = crate::screen::core::capture_test_support::image(
        &BgraImage::new(NATIVE).expect("image"),
        &grant,
    );
    let cpu_picture = Picture::Cpu(pixels.clone());
    assert!(cpu_picture.to_cpu().expect("cpu").shares_pixels(&pixels));
    // A GPU picture larger than its surface (beyond the NV12 rounding) is refused.
    let too_large = FrameSize::new(NATIVE.width + 2, NATIVE.height);
    assert!(GpuPicture::new(surface, too_large, FrameLimits::LEGACY, &grant).is_err());
}
