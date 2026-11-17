//! Task tests W4 (review B3): full encoder rank fallback in worker and daemon.
use std::borrow::Cow;
use std::io;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::encode::{EncoderOpener, Encoding};
use crate::screen::core::{
    capture_admission::{
        CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SessionAdmission, SourceKey,
    },
    capture_image::{AdmittedBytes, CaptureImageBuilder, CapturedImage},
    capture_picture::Picture,
    capture_plan::CapturePhases,
    capture_records::CapturedRecords,
    codec_registry::{CodecRegistry, EncoderFactory, OpenedEncoder},
    frame_budget::FrameLimits,
    gop_continue::ContinueOffers,
    gop_stream::{accept_picture_streams_with_contract, PictureEvent},
    host_backend::{
        CapturedEncodedFrame, EncodedFrame, ScreenEncoder, ScreenHostBackend, ScreenSource,
        SourceFrame, SourceRefusal,
    },
    host_pipeline::{codec_fallbacks, encode_stage_frame, Pipeline, Produced, TestStage},
    host_stage::StageRequest,
    input_model::{InputCapabilities, InputEvent},
    negotiate::{negotiate, CodecChoice, CodecKind, CodecOffer, CodecPreference, H264Profile},
    pixel_convert::{BgraImage, FrameSize},
    set_decoders::SetDecoders,
    window_list::{MonitorInfo, SourceId, WindowList},
    wire_control::ControlBlock,
    wire_hello::HostPlatform,
};
use crate::screen::os::synthetic_backend::SyntheticBackend;

const SIZE: FrameSize = FrameSize::new(64, 64);
const ALPN: &[u8] = b"bildkette-w4-rank/1";

#[derive(Default)]
struct CallLog {
    opens: Mutex<Vec<String>>,
    encodes: Mutex<Vec<(String, u64, bool)>>,
}

struct MockFactory {
    name: &'static str,
    offers: Vec<CodecOffer>,
    succeed_calls: usize,
    log: Arc<CallLog>,
}

struct MockEncoder {
    name: &'static str,
    size: FrameSize,
    remaining_ok: usize,
    log: Arc<CallLog>,
}

impl ScreenEncoder for MockEncoder {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let entry = (self.name.into(), request.frame_id, request.keyframe);
        self.log.encodes.lock().unwrap().push(entry);
        if self.remaining_ok == 0 {
            return Err(io::Error::other(format!("{} driver hung", self.name)));
        }
        self.remaining_ok -= 1;
        let mut bytes = AdmittedBytes::buffer(&frame.grant(), CaptureOwner::CodecOutput, 8)?;
        bytes.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88])?;
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::one(bytes)?,
            keyframe: request.keyframe,
        })
    }
    fn name(&self) -> String {
        self.name.into()
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("admitted path only"))
    }
}

impl EncoderFactory for MockFactory {
    fn name(&self) -> String {
        self.name.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        self.offers.clone()
    }
    fn open(&self, _: &CodecChoice, size: FrameSize, _: u32) -> io::Result<Box<dyn ScreenEncoder>> {
        self.log.opens.lock().unwrap().push(self.name.into());
        Ok(Box::new(MockEncoder {
            name: self.name,
            size,
            remaining_ok: self.succeed_calls,
            log: self.log.clone(),
        }))
    }
    fn open_admitted(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        _grant: CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        self.open(choice, size, fps)
    }
}

fn mock(
    name: &'static str,
    offers: Vec<CodecOffer>,
    succeed_calls: usize,
    log: &Arc<CallLog>,
) -> Arc<dyn EncoderFactory> {
    Arc::new(MockFactory {
        name,
        offers,
        succeed_calls,
        log: log.clone(),
    })
}

struct RegistryOpener(CodecRegistry);

impl EncoderOpener for RegistryOpener {
    fn open(
        &self,
        request: &StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        self.0
            .open_encoder_for_stage_admitted(request, refused, grant)
    }
    fn stage_count(&self, request: &StageRequest) -> usize {
        self.0.encoder_stage_count(request)
    }
}

fn h264_high_choice() -> CodecChoice {
    CodecChoice {
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
        encoder_hardware: true,
        decoder_hardware: true,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}

fn stage_request(frame_id: u64, keyframe: bool, choice: CodecChoice) -> StageRequest {
    StageRequest {
        frame_id,
        frame_contract: None,
        visible_rects: None,
        keyframe,
        target_bps: 4_000_000,
        size: SIZE,
        choice,
        encoder_epoch: 1,
        fps: 60,
    }
}

fn test_grant() -> (Arc<SessionAdmission>, CaptureGrant) {
    let admission = crate::screen::core::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "w4-rank").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.bind_contract(None).unwrap();
    for owner in [
        CaptureOwner::SourcePixels,
        CaptureOwner::ScaledPixels,
        CaptureOwner::CodecInput,
    ] {
        plan.add_bgra(owner, SIZE, 2, CapturePhases::ALL).unwrap();
    }
    // `Metadata`: the record headers of every encoded frame (`CapturedRecords`).
    for owner in [
        CaptureOwner::CodecOutput,
        CaptureOwner::FilterScratch,
        CaptureOwner::Metadata,
    ] {
        plan.add_bytes(owner, 65_536, 2, CapturePhases::ALL)
            .unwrap();
    }
    let grant = admission.authority().approve(plan).unwrap();
    (admission, grant)
}

fn captured_image(grant: &CaptureGrant) -> CapturedImage {
    CaptureImageBuilder::new(SIZE, FrameLimits::LEGACY, grant, CaptureOwner::SourcePixels)
        .unwrap()
        .freeze()
}

fn windows_h264_rank(
    succeed_at: Option<&str>,
    nvenc_ok_frames: usize,
    log: &Arc<CallLog>,
) -> CodecRegistry {
    let ok = |name: &str| match () {
        _ if Some(name) == succeed_at => usize::MAX,
        _ if name == "nvenc" => nvenc_ok_frames,
        _ => 0,
    };
    let high_hw = CodecOffer::h264(H264Profile::High, true);
    let cb_hw = CodecOffer::h264(H264Profile::ConstrainedBaseline, true);
    let high_sw = CodecOffer::h264(H264Profile::High, false);
    let cb_sw = CodecOffer::h264(H264Profile::ConstrainedBaseline, false)
        .with_max(FrameSize::new(3840, 2160));
    CodecRegistry::new(
        vec![
            mock("nvenc", vec![high_hw], ok("nvenc"), log),
            mock("amf", vec![high_hw], ok("amf"), log),
            mock("mf-hardware", vec![high_hw, cb_hw], ok("mf-hardware"), log),
            // Duplicate factory name (second identical adapter MFT): never tried twice.
            mock("mf-hardware", vec![high_hw], usize::MAX, log),
            mock("mf-software", vec![high_sw, cb_sw], ok("mf-software"), log),
            mock("openh264", vec![cb_sw], ok("openh264"), log),
        ],
        Vec::new(),
    )
}

#[test]
fn bildkette_task_win_w4_worker_rank_tries_every_stage_once_through_openh264() {
    let (_admission, grant) = test_grant();
    let image = Picture::Cpu(captured_image(&grant));
    let req = stage_request(1, true, h264_high_choice());

    let log = Arc::new(CallLog::default());
    let registry = windows_h264_rank(Some("openh264"), 0, &log);
    assert_eq!(registry.encoder_stage_count(&req), 5);
    let mut encoding = Encoding::new(Arc::new(RegistryOpener(registry)));
    let (frame, name, hardware) = encoding
        .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
        .unwrap();
    assert!(frame.keyframe && !hardware && name == "openh264");
    let expected_rank = ["nvenc", "amf", "mf-hardware", "mf-software", "openh264"];
    assert_eq!(*log.opens.lock().unwrap(), expected_rank);
    assert_eq!(encoding.refused, expected_rank[..4]);

    let log = Arc::new(CallLog::default());
    let registry = windows_h264_rank(Some("mf-software"), 1, &log);
    let mut encoding = Encoding::new(Arc::new(RegistryOpener(registry)));
    let (_, first_name, _) = encoding
        .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
        .unwrap();
    assert_eq!(first_name, "nvenc");
    let delta = stage_request(2, false, h264_high_choice());
    let (second, second_name, second_hw) = encoding
        .encode_with_fallback(&image, Cow::Borrowed(&delta), &grant)
        .unwrap();
    assert!(second.keyframe && !second_hw && second_name == "mf-software");
    let calls = [
        ("nvenc".into(), 1, true),
        ("nvenc".into(), 2, false),
        ("amf".into(), 2, true),
        ("mf-hardware".into(), 2, true),
        ("mf-software".into(), 2, true),
    ];
    assert_eq!(*log.encodes.lock().unwrap(), calls);

    let log = Arc::new(CallLog::default());
    let mut encoding = Encoding::new(Arc::new(RegistryOpener(windows_h264_rank(None, 0, &log))));
    assert!(encoding
        .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
        .is_err());
    assert_eq!(encoding.refused, expected_rank);
    assert_eq!(log.opens.lock().unwrap().len(), 5);
    assert!(encoding
        .encode_with_fallback(&image, Cow::Borrowed(&req), &grant)
        .is_err());
    assert_eq!(log.opens.lock().unwrap().len(), 5);
}

struct RankSource(CaptureGrant);

impl ScreenSource for RankSource {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.0.clone())
    }
    fn capture(&mut self, _: u64, _: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Frame(captured_image(&self.0)))
    }
}

struct RankBackend(CodecRegistry);

impl ScreenHostBackend for RankBackend {
    fn platform(&self) -> HostPlatform {
        SyntheticBackend.platform()
    }
    fn graphical_session(&self) -> bool {
        true
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        SyntheticBackend.monitors()
    }
    fn encoders(&self) -> Vec<CodecOffer> {
        self.0.encoder_offers()
    }
    fn input(&self) -> InputCapabilities {
        SyntheticBackend.input()
    }
    fn limits(&self) -> Vec<String> {
        SyntheticBackend.limits()
    }
    fn windows(&self) -> io::Result<WindowList> {
        SyntheticBackend.windows()
    }
    fn open_source(
        &self,
        s: &SourceId,
        w: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        SyntheticBackend.open_source(s, w)
    }
    fn open_encoder(&self, c: &CodecChoice, s: FrameSize) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(self.0.open_encoder(c, s, 60)?.encoder)
    }
    fn open_stage_encoder(
        &self,
        req: &StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        self.0.open_encoder_for_stage_admitted(req, refused, grant)
    }
    fn encoder_stages(&self, request: &StageRequest) -> usize {
        self.0.encoder_stage_count(request)
    }
    fn inject(&self, _: &[InputEvent]) -> Result<(), ControlBlock> {
        Ok(())
    }
}

async fn loopback() -> (
    iroh::Endpoint,
    iroh::Endpoint,
    iroh::endpoint::Connection,
    iroh::endpoint::Connection,
) {
    async fn bind() -> iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap()
    }
    let (left, right) = (bind().await, bind().await);
    let addr = iroh::EndpointAddr::from_parts(
        right.id(),
        right
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (out, inc) = tokio::join!(left.connect(addr, ALPN), async {
        right.accept().await.unwrap().await
    });
    (left, right, out.unwrap(), inc.unwrap())
}

#[test]
fn bildkette_task_win_w4_daemon_stage_and_pipeline_complete_rank_to_tiles() {
    let (_admission, grant) = test_grant();
    let img = captured_image(&grant);
    let req = stage_request(1, true, h264_high_choice());

    // 1. Daemon in-process `Stage` walks NVENC -> AMF -> MF-HW -> MF-SW -> OpenH264.
    let log = Arc::new(CallLog::default());
    let backend = RankBackend(windows_h264_rank(Some("openh264"), 0, &log));
    let mut stage = TestStage {
        source: Box::new(RankSource(grant.clone())),
        encoder: None,
        encoder_book: None,
        refused: Vec::new(),
        memory: None,
        chain: Default::default(),
    };
    let (frame, info, _) =
        encode_stage_frame(&mut stage, &backend, &img, Cow::Borrowed(&req)).unwrap();
    assert!(frame.keyframe && !info.hardware && info.name == "openh264");
    assert_eq!(
        stage.refused,
        ["nvenc", "amf", "mf-hardware", "mf-software"]
    );

    // 2. Full Pipeline chain: AV1 -> HEVC -> H.264 (all 5 stages) -> Tiles.
    let full_log = Arc::new(CallLog::default());
    let high_hw = CodecOffer::h264(H264Profile::High, true);
    let high_sw = CodecOffer::h264(H264Profile::High, false);
    let cb_sw = CodecOffer::h264(H264Profile::ConstrainedBaseline, false);
    let av1_hw = CodecOffer::video(CodecKind::Av1, false, true);
    let hevc_hw = CodecOffer::video(CodecKind::Hevc, false, true);
    let factories = vec![
        mock("nvenc-av1", vec![av1_hw], 0, &full_log),
        mock("nvenc-hevc", vec![hevc_hw], 0, &full_log),
        mock("nvenc", vec![high_hw], 0, &full_log),
        mock("amf", vec![high_hw], 0, &full_log),
        mock("mf-hardware", vec![high_hw], 0, &full_log),
        mock("mf-software", vec![high_sw], 0, &full_log),
        mock("openh264", vec![cb_sw], 0, &full_log),
    ];
    let backend = Arc::new(RankBackend(CodecRegistry::new(factories, Vec::new())));
    let viewer_offers = vec![av1_hw, hevc_hw, high_hw, CodecOffer::tiles()];
    let choices = negotiate(&backend.encoders(), &viewer_offers, CodecPreference::Auto);
    assert_eq!(choices[0].codec, CodecKind::Av1);
    let fallbacks = codec_fallbacks(&choices, &choices[0]);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (left, right, host, viewer) = loopback().await;
            let (events, mut rx) = tokio::sync::mpsc::channel(8);
            let (acks, _ack_rx) = tokio::sync::mpsc::channel(8);
            let reader = tokio::spawn(accept_picture_streams_with_contract(
                viewer.clone(),
                events,
                acks,
                None,
            ));
            let source = Box::new(RankSource(grant.clone()));
            let resets = Arc::new(AtomicU32::new(0));
            let mut pipeline = Pipeline::new(
                backend.clone(),
                source,
                choices[0],
                &host,
                30,
                Some(SIZE),
                resets,
            );
            pipeline.set_pairing(backend.encoders(), None, ContinueOffers::default());
            pipeline.set_codec_fallbacks(fallbacks);

            async fn step(pipeline: &mut Pipeline, host: &iroh::endpoint::Connection) -> Produced {
                loop {
                    match pipeline.tick(host).await.unwrap() {
                        Produced::Idle => tokio::time::sleep(Duration::from_millis(5)).await,
                        other => return other,
                    }
                }
            }

            for _ in 0..3 {
                assert_eq!(step(&mut pipeline, &host).await, Produced::Reset);
            }
            assert_eq!(step(&mut pipeline, &host).await, Produced::Sent);
            assert_eq!(
                *full_log.opens.lock().unwrap(),
                [
                    "nvenc-av1",
                    "nvenc-hevc",
                    "nvenc",
                    "amf",
                    "mf-hardware",
                    "mf-software",
                    "openh264",
                ]
            );
            let PictureEvent::Record { header, .. } = rx.recv().await.unwrap() else {
                panic!("expected a picture record");
            };
            assert_eq!(header.codec, CodecKind::Tiles);

            let _ = pipeline.set_decoders(&SetDecoders {
                decoders: viewer_offers.clone(),
                ..SetDecoders::default()
            });
            assert_eq!(full_log.opens.lock().unwrap().len(), 7);
            drop(pipeline);
            host.close(0u32.into(), b"done");
            viewer.close(0u32.into(), b"done");
            reader.abort();
            let _ = (reader.await, left.close().await, right.close().await);
        })
        .await
        .expect("w4 pipeline fallback deadline");
    });
}
