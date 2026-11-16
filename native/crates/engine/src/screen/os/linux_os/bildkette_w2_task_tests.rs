//! Task suite „Bildkette“ merge W2 (Linux): a DMA-BUF picture of the
//! capture reaches the VA encoder as a GPU picture without a CPU consumer,
//! every failure of that route ends DMA-BUF for the session and keeps the
//! encoder, the VA rate ceiling comes from the level and the quality level
//! is the driver's fastest, and NVENC continues after a reset without a key
//! frame. No GPU, no compositor: a test session stands for the VA session,
//! a test stream for the PipeWire stream. Not registered yet
//! (`docs/tests/register.tsv`): the manager adds the rows.
use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::hw_dmabuf_picture::{
    cause_of, ConversionFailed, DmaBufCause, DmaBufPicture, DmaBufStream,
};
use super::hw_encode_common::{
    avc_level_ceiling, HeldDmaBuf, HwH264Encoder, HwH264Session, HwInput, RateParams,
};
use super::vaapi_dmabuf::DRM_XRGB8888;
use super::vaapi_vpp::DmaBufFrame;
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant, SourceKey};
use crate::screen::core::capture_picture::{GpuPicture, Picture};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::{FrameSize, MAX_FRAME_PIXELS};
use crate::screen::core::ref_recovery::{Recovery, RefHistory};

const NATIVE: FrameSize = FrameSize::new(64, 36);
const SMALL: FrameSize = FrameSize::new(32, 18);

/// The PipeWire stream: records why the GPU route ended.
#[derive(Default)]
struct Stream(Mutex<Vec<DmaBufCause>>);

impl DmaBufStream for Stream {
    fn fall_back(&self, cause: DmaBufCause) {
        self.0.lock().unwrap().push(cause);
    }
}

impl Stream {
    fn causes(&self) -> Vec<DmaBufCause> {
        self.0.lock().unwrap().clone()
    }
}

/// A held PipeWire buffer: counts its return to the compositor.
struct Held {
    frame: DmaBufFrame,
    source: SourceKey,
    generation: CaptureGeneration,
    returned: Arc<AtomicUsize>,
}

impl HeldDmaBuf for Held {
    fn frame(&self) -> &DmaBufFrame {
        &self.frame
    }
    fn identity(&self) -> (&SourceKey, CaptureGeneration) {
        (&self.source, self.generation)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.returned.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum Import {
    Works,
    Fails,
    ConversionFails,
}

/// The VA session: logs what reached it, encodes nothing.
struct Session {
    imports: bool,
    import: Import,
    log: Arc<Mutex<Vec<String>>>,
    level: Option<u64>,
}

impl HwH264Session for Session {
    fn wants_bgra(&self) -> bool {
        true
    }
    fn imports_dmabuf(&self) -> bool {
        self.imports
    }
    fn import_dmabuf(&mut self, frame: &DmaBufFrame) -> io::Result<()> {
        let size = frame.size;
        let entry = format!("import {}x{}", size.width, size.height);
        self.log.lock().unwrap().push(entry);
        match self.import {
            Import::Works => Ok(()),
            Import::Fails => Err(io::Error::other("vaCreateSurfaces refused")),
            Import::ConversionFails => Err(io::Error::other(ConversionFailed("vpp".into()))),
        }
    }
    fn set_frame_id(&mut self, frame_id: u64) {
        self.log.lock().unwrap().push(format!("frame {frame_id}"));
    }
    fn recover_references(&mut self, last_good: u64) -> Recovery {
        self.log
            .lock()
            .unwrap()
            .push(format!("recover {last_good}"));
        Recovery::Continued
    }
    fn level_ceiling(&self, _fps: u32) -> Option<u64> {
        self.level
    }
    fn encode(
        &mut self,
        input: HwInput<'_>,
        _idr: bool,
        _rate: &RateParams,
        _deliver: &mut dyn FnMut(&[&[u8]]) -> io::Result<()>,
    ) -> io::Result<()> {
        let kind = match input {
            HwInput::Prepared => "prepared",
            HwInput::Bgra(_) => "bgra",
            HwInput::I420(_) => "i420",
        };
        self.log.lock().unwrap().push(format!("encode {kind}"));
        Err(io::Error::other("test session encodes nothing"))
    }
}

fn session(imports: bool, import: Import) -> (Session, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let session = Session {
        imports,
        import,
        log: log.clone(),
        level: None,
    };
    (session, log)
}

fn request(size: FrameSize, frame_id: u64) -> StageRequest {
    StageRequest {
        frame_id,
        frame_contract: None,
        visible_rects: None,
        keyframe: false,
        target_bps: 4_000_000,
        size,
        choice: CodecChoice {
            codec: CodecKind::H264,
            profile: Some(H264Profile::High),
            encoder_hardware: true,
            decoder_hardware: true,
            max_size: FrameSize::MAX,
            max_pixels: MAX_FRAME_PIXELS,
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 1,
        fps: 60,
    }
}

/// A DMA-BUF picture of `grant`'s source; `held` gives a buffer to the
/// encoder, else it is a repeat. The render node does not exist, so every
/// read-back fails: a read-back shows as an error and a `Consumer` cause.
fn picture(
    grant: &CaptureGrant,
    held: bool,
    stream: &Arc<Stream>,
    returned: &Arc<AtomicUsize>,
) -> (Picture, Arc<DmaBufPicture>) {
    let frame = DmaBufFrame {
        fd: -1,
        drm_format: DRM_XRGB8888,
        modifier: 0,
        size: NATIVE,
        planes: vec![(0, NATIVE.width * 4)],
        object_size: NATIVE.width * 4 * NATIVE.height,
    };
    let held: Option<Box<dyn HeldDmaBuf + Send>> = held.then(|| {
        Box::new(Held {
            frame: frame.clone(),
            source: grant.source().clone(),
            generation: grant.generation(),
            returned: returned.clone(),
        }) as Box<dyn HeldDmaBuf + Send>
    });
    let descriptor = Arc::new(OwnedFd::from(std::fs::File::open("/dev/null").unwrap()));
    let node = PathBuf::from("/nonexistent/se-w2-renderD128");
    let stream: Arc<dyn DmaBufStream> = stream.clone();
    let surface = Arc::new(DmaBufPicture::new(
        held,
        (frame, descriptor),
        node,
        stream,
        7,
    ));
    let gpu = GpuPicture::new(surface.clone(), NATIVE, FrameLimits::RESOURCE, grant).unwrap();
    (Picture::Gpu(gpu), surface)
}

fn encoder(session: Session, grant: &CaptureGrant) -> HwH264Encoder<Session> {
    HwH264Encoder::new(session, "vaapi-test".into(), SMALL, 60, Some(grant.clone()))
}

#[test]
fn bildkette_task_linux_w2_gpu_picture_reaches_the_encoder_without_a_cpu_consumer() {
    let grant = crate::screen::core::capture_test_support::grant(NATIVE, None);
    let stream = Arc::new(Stream::default());
    let returned = Arc::new(AtomicUsize::new(0));
    let (picture, surface) = self::picture(&grant, true, &stream, &returned);
    // The host process relabels the size; no pixel work, no read-back.
    let scaled = picture
        .scaled(
            SMALL,
            FrameLimits::RESOURCE,
            crate::screen::core::capture_admission::CaptureOwner::ScaledPixels,
        )
        .unwrap();
    assert!(scaled.on_gpu());
    let (session, log) = self::session(true, Import::Works);
    let mut encoder = self::encoder(session, &grant);
    assert!(encoder.takes_dmabuf());
    let error = encoder
        .encode_picture(&scaled, &request(SMALL, 77))
        .unwrap_err();
    assert_eq!(error.to_string(), "test session encodes nothing");
    // Imported at its own size (the video processor scales), the buffer
    // went back after the import, the input was encoded as it lay.
    assert_eq!(
        *log.lock().unwrap(),
        ["import 64x36", "frame 77", "encode prepared"]
    );
    assert_eq!(returned.load(Ordering::SeqCst), 1);
    assert!(!surface.holds_buffer());
    // No CPU consumer: nothing was read back, DMA-BUF stays.
    assert!(stream.causes().is_empty());
    // B-36 reaches the session through the encoder.
    assert_eq!(encoder.recover_references(76), Recovery::Continued);
    assert_eq!(log.lock().unwrap().last().unwrap(), "recover 76");
}

#[test]
fn bildkette_task_linux_w2_dmabuf_failure_falls_back_to_shared_memory() {
    let grant = crate::screen::core::capture_test_support::grant(NATIVE, None);
    for (import, cause) in [
        (Import::Fails, DmaBufCause::Import),
        (Import::ConversionFails, DmaBufCause::Conversion),
    ] {
        let stream = Arc::new(Stream::default());
        let returned = Arc::new(AtomicUsize::new(0));
        let (picture, _) = self::picture(&grant, true, &stream, &returned);
        let (session, log) = self::session(true, import);
        let mut encoder = self::encoder(session, &grant);
        let error = encoder
            .encode_picture(&picture, &request(SMALL, 1))
            .unwrap_err();
        // The picture is skipped, the encoder is not refused (no error of
        // the encoder), the stream leaves DMA-BUF for the session.
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(stream.causes(), [cause]);
        assert_eq!(returned.load(Ordering::SeqCst), 1);
        assert_eq!(*log.lock().unwrap(), ["import 64x36"]);
    }
    // An encoder without import (NVENC, no video processor) gets the
    // picture only read back, and the stream returns to shared memory.
    let stream = Arc::new(Stream::default());
    let returned = Arc::new(AtomicUsize::new(0));
    let (picture, _) = self::picture(&grant, true, &stream, &returned);
    let (session, log) = self::session(false, Import::Works);
    let mut encoder = self::encoder(session, &grant);
    assert!(!encoder.takes_dmabuf());
    assert!(encoder
        .encode_picture(&picture, &request(SMALL, 1))
        .is_err());
    assert_eq!(stream.causes(), [DmaBufCause::Consumer]);
    assert_eq!(returned.load(Ordering::SeqCst), 1);
    assert!(
        log.lock().unwrap().is_empty(),
        "no import without the route"
    );
    // A key frame of a still screen for a new encoder reads back once
    // without leaving DMA-BUF (here the read-back fails: no device).
    let stream = Arc::new(Stream::default());
    let (repeat, _) = self::picture(&grant, false, &stream, &returned);
    let (session, log) = self::session(true, Import::Works);
    let mut encoder = self::encoder(session, &grant);
    assert!(encoder.encode_picture(&repeat, &request(SMALL, 2)).is_err());
    assert!(stream.causes().is_empty());
    assert!(log.lock().unwrap().is_empty());
    // The cause of an import step.
    assert_eq!(cause_of(&io::Error::other("refused")), DmaBufCause::Import);
    let conversion = io::Error::other(ConversionFailed("vpp".into()));
    assert_eq!(cause_of(&conversion), DmaBufCause::Conversion);
}

#[test]
fn bildkette_task_linux_w2_va_rate_ceiling_comes_from_the_level() {
    use super::vaapi_encode_base::fastest_quality_level;
    use super::vaapi_ffi::{VA_ATTRIB_NOT_SUPPORTED, VA_CONFIG_ATTRIB_ENC_QUALITY_RANGE};
    use super::vaapi_h264_abi::{VaEncMiscQualityLevel, VA_ENC_MISC_QUALITY_LEVEL};
    use crate::screen::core::rate_control::avc_level_max_bitrate_bps;
    // No fixed ceiling in the rate parameters any more (was 240 Mbit/s).
    assert_eq!(
        RateParams::for_target(u64::MAX, 30).bits_per_second,
        u32::MAX
    );
    assert_eq!(RateParams::for_target(1_000, 30).bits_per_second, 150_000);
    // H.264: MaxBR of the level the session writes (Table A-1, High 1250).
    assert_eq!(
        avc_level_max_bitrate_bps(40, H264Profile::High),
        Some(25_000_000)
    );
    assert_eq!(
        avc_level_max_bitrate_bps(41, H264Profile::ConstrainedBaseline),
        Some(50_000_000)
    );
    assert_eq!(avc_level_max_bitrate_bps(99, H264Profile::High), None);
    let full_hd = FrameSize::new(1920, 1080);
    assert_eq!(
        avc_level_ceiling(full_hd, 30, H264Profile::High),
        Some(25_000_000)
    );
    assert_eq!(
        avc_level_ceiling(full_hd, 60, H264Profile::High),
        Some(62_500_000)
    );
    // The encoder takes the lower of the pairing's and the level's ceiling.
    let grant = crate::screen::core::capture_test_support::grant(NATIVE, None);
    let (mut test_session, _) = self::session(true, Import::Works);
    test_session.level = Some(25_000_000);
    let encoder = encoder(test_session, &grant).with_ceiling(800_000_000);
    assert_eq!(encoder.ceiling(), 25_000_000);
    let (test_session, _) = self::session(true, Import::Works);
    let encoder = HwH264Encoder::new(test_session, "t".into(), SMALL, 60, None);
    assert_eq!(encoder.with_ceiling(10_000_000).ceiling(), 10_000_000);
    // The fastest quality level (va.h: 1 best quality, the range fastest).
    assert_eq!(VA_CONFIG_ATTRIB_ENC_QUALITY_RANGE, 21);
    assert_eq!(VA_ENC_MISC_QUALITY_LEVEL, 6);
    assert_eq!(std::mem::size_of::<VaEncMiscQualityLevel>(), 20);
    assert_eq!(fastest_quality_level(7), Some(7));
    assert_eq!(fastest_quality_level(1), None);
    assert_eq!(fastest_quality_level(0), None);
    assert_eq!(fastest_quality_level(VA_ATTRIB_NOT_SUPPORTED), None);
    // Every VA session sends it with each sequence.
    for (name, text) in [
        ("vaapi_encode.rs", include_str!("vaapi_encode.rs")),
        ("vaapi_hevc_encode.rs", include_str!("vaapi_hevc_encode.rs")),
        ("vaapi_av1_encode.rs", include_str!("vaapi_av1_encode.rs")),
    ] {
        assert!(
            text.contains("self.base.add_quality(&mut buffers)?;"),
            "{name}"
        );
        assert!(text.contains("fn level_ceiling(&self, fps: u32)"), "{name}");
    }
}

#[test]
fn bildkette_task_linux_w2_nvenc_recovers_without_a_key_frame() {
    use super::nvenc_ffi::{NvEncConfig, Zeroed, NV_ENC_NUM_REF_FRAMES_1};
    use crate::screen::os::nvenc_shared::codecs::{av1_view, hevc_view};
    use crate::screen::os::nvenc_shared::settings::{
        apply_codec_references, apply_codec_settings, RFI_DPB,
    };
    let rate = RateParams::for_target(20_000_000, 60);
    // The shared settings reach Linux: one reference, DPB 5 (AV1 8),
    // low-delay key frame scale, only with reference invalidation.
    let mut config = NvEncConfig::zeroed();
    apply_codec_settings(&mut config, &rate, CodecKind::H264);
    apply_codec_references(&mut config, CodecKind::H264, true);
    assert_eq!(config.rc_params.average_bit_rate, 20_000_000);
    assert_eq!(config.rc_params.low_delay_key_frame_scale, 1);
    assert_eq!(config.h264.max_num_ref_frames, RFI_DPB);
    assert_eq!(config.h264.num_ref_l0, NV_ENC_NUM_REF_FRAMES_1);
    let mut without = NvEncConfig::zeroed();
    apply_codec_references(&mut without, CodecKind::H264, false);
    assert_eq!(without.h264.max_num_ref_frames, 0);
    let mut hevc = NvEncConfig::zeroed();
    apply_codec_settings(&mut hevc, &rate, CodecKind::Hevc);
    apply_codec_references(&mut hevc, CodecKind::Hevc, true);
    assert_eq!(hevc_view(&mut hevc).max_num_ref_frames_in_dpb, 5);
    let mut av1 = NvEncConfig::zeroed();
    apply_codec_settings(&mut av1, &rate, CodecKind::Av1);
    apply_codec_references(&mut av1, CodecKind::Av1, true);
    assert_eq!(av1_view(&mut av1).max_num_ref_frames_in_dpb, 8);
    // The bookkeeping of the session: frame ids as `inputTimeStamp`; the
    // pictures after the confirmed one are invalidated, none is left when
    // the confirmed one fell out of the DPB (then a key frame).
    let mut history = RefHistory::new(RFI_DPB);
    for (frame, idr) in [(10, true), (11, false), (12, false), (13, false)] {
        history.encoded(frame, idr);
    }
    assert_eq!(history.invalidate_after(11), Some(vec![12, 13]));
    history.encoded(14, false);
    assert_eq!(history.invalidate_after(9), None);
    // The Linux session stamps and invalidates as Windows' does, and the
    // host process hands the encoder the id the viewer acknowledges.
    let linux = include_str!("nvenc_encode.rs");
    for needle in [
        "picture.input_time_stamp = self.frame_id;",
        "self.history.encoded(self.frame_id, idr_coded || idr);",
        "fn recover_references(&mut self, last_good: u64) -> Recovery {",
        "self.history.invalidate_after(last_good)",
        "apply_codec_references(&mut session.config, codec, invalidation);",
    ] {
        assert!(linux.contains(needle), "{needle}");
    }
    let worker = include_str!("../shared/host_worker_encode.rs");
    assert!(worker.contains("request.to_mut().frame_id = frame_id;"));
    // Both platforms use the one shared module.
    let windows = include_str!("../windows/nvenc_encode.rs");
    assert!(windows.contains("crate::screen::os::nvenc_shared::settings"));
    assert!(linux.contains("crate::screen::os::nvenc_shared::settings"));
}
