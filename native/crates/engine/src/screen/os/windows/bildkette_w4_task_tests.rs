//! Task tests W4 (plan 2026-11-14 „Bildkette“, reviews B1 and B3):
//! asynchronous NVENC completion events with a bounded wait (`max(4 frame
//! intervals, 100 ms)`) and rank fallback without blocking in
//! `nvEncLockBitstream`.
use std::borrow::Cow;
use std::ffi::{c_int, c_void};
use std::io;
use std::mem::{offset_of, size_of};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_sys::Win32::System::Threading::SetEvent;

use super::nvenc_encode::{async_timeout_ms, Core, MIN_ASYNC_TIMEOUT_MS};
use super::nvenc_ffi::{self as nv, NvTable, Zeroed};
use super::vendor_encode::RateParams;
use super::*;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SourceKey,
};
use crate::screen::core::capture_image::{AdmittedBytes, CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::capture_records::CapturedRecords;
use crate::screen::core::codec_registry::{CodecRegistry, EncoderFactory, OpenedEncoder};
use crate::screen::core::frame_budget::{FrameLimits, MAX_WORKING_BYTES};
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::os::host_worker_main::{EncoderOpener, Encoding};

struct FakeDriver {
    async_supported: bool,
    signal_event: bool,
    async_init: u32,
    async_reinit: u32,
    registered_event: nv::Ptr,
    unregistered_event: bool,
    bitstream_destroyed: bool,
    destroyed: bool,
    lock_calls: u32,
    last_lock_flags: u32,
    payload: [u8; 6],
}

impl FakeDriver {
    fn new(async_supported: bool, signal_event: bool) -> Self {
        Self {
            async_supported,
            signal_event,
            async_init: 0,
            async_reinit: 0,
            registered_event: std::ptr::null_mut(),
            unregistered_event: false,
            bitstream_destroyed: false,
            destroyed: false,
            lock_calls: 0,
            last_lock_flags: 0,
            payload: [0, 0, 0, 1, 0x65, 0x88],
        }
    }
}

unsafe extern "system" fn fake_get_caps(
    encoder: nv::Ptr,
    _: nv::Guid,
    param: *mut nv::NvEncCapsParam,
    out: *mut c_int,
) -> nv::NvEncStatus {
    let driver = &*(encoder as *const FakeDriver);
    *out = match (*param).caps_to_query {
        nv::NV_ENC_CAPS_ASYNC_ENCODE_SUPPORT => c_int::from(driver.async_supported),
        nv::NV_ENC_CAPS_WIDTH_MAX | nv::NV_ENC_CAPS_HEIGHT_MAX => 4096,
        nv::NV_ENC_CAPS_SUPPORT_REF_PIC_INVALIDATION
        | nv::NV_ENC_CAPS_SUPPORT_MULTIPLE_REF_FRAMES => 1,
        _ => 0,
    };
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_preset(
    _: nv::Ptr,
    _: nv::Guid,
    _: nv::Guid,
    _: u32,
    config: *mut nv::NvEncPresetConfig,
) -> nv::NvEncStatus {
    (*config).preset_cfg.version = nv::NV_ENC_CONFIG_VER;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_init(
    encoder: nv::Ptr,
    params: *mut nv::NvEncInitializeParams,
) -> nv::NvEncStatus {
    (*(encoder as *mut FakeDriver)).async_init = (*params).enable_encode_async;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_reconfigure(
    encoder: nv::Ptr,
    params: *mut nv::NvEncReconfigureParams,
) -> nv::NvEncStatus {
    (*(encoder as *mut FakeDriver)).async_reinit =
        (*params).re_init_encode_params.enable_encode_async;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_register_event(
    encoder: nv::Ptr,
    params: *mut nv::NvEncEventParams,
) -> nv::NvEncStatus {
    assert_eq!((*params).version, nv::NV_ENC_EVENT_PARAMS_VER);
    assert!(!(*params).completion_event.is_null());
    (*(encoder as *mut FakeDriver)).registered_event = (*params).completion_event;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_unregister_event(
    encoder: nv::Ptr,
    params: *mut nv::NvEncEventParams,
) -> nv::NvEncStatus {
    assert_eq!((*params).version, nv::NV_ENC_EVENT_PARAMS_VER);
    let driver = &mut *(encoder as *mut FakeDriver);
    assert_eq!((*params).completion_event, driver.registered_event);
    driver.unregistered_event = true;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_create_bitstream(
    _: nv::Ptr,
    params: *mut nv::NvEncCreateBitstreamBuffer,
) -> nv::NvEncStatus {
    (*params).bitstream_buffer = 0x1000usize as nv::Ptr;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_destroy_bitstream(
    encoder: nv::Ptr,
    buffer: nv::Ptr,
) -> nv::NvEncStatus {
    assert_eq!(buffer, 0x1000usize as nv::Ptr);
    (*(encoder as *mut FakeDriver)).bitstream_destroyed = true;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_encode(
    encoder: nv::Ptr,
    params: *mut nv::NvEncPicParams,
) -> nv::NvEncStatus {
    let driver = &mut *(encoder as *mut FakeDriver);
    assert_eq!((*params).completion_event, driver.registered_event);
    if driver.signal_event {
        assert_ne!(SetEvent((*params).completion_event), 0);
    }
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_lock(
    encoder: nv::Ptr,
    params: *mut nv::NvEncLockBitstream,
) -> nv::NvEncStatus {
    let driver = &mut *(encoder as *mut FakeDriver);
    driver.lock_calls += 1;
    driver.last_lock_flags = (*params).flags;
    (*params).bitstream_buffer_ptr = driver.payload.as_mut_ptr().cast::<c_void>();
    (*params).bitstream_size_in_bytes = driver.payload.len() as u32;
    (*params).picture_type = nv::NV_ENC_PIC_TYPE_IDR;
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_unlock(_: nv::Ptr, _: nv::Ptr) -> nv::NvEncStatus {
    nv::NV_ENC_SUCCESS
}

unsafe extern "system" fn fake_destroy_encoder(encoder: nv::Ptr) -> nv::NvEncStatus {
    (*(encoder as *mut FakeDriver)).destroyed = true;
    nv::NV_ENC_SUCCESS
}

struct FakeTable(nv::NvEncFunctions);

impl FakeTable {
    fn new() -> Self {
        let mut f = nv::NvEncFunctions::zeroed();
        f.version = nv::NV_ENCODE_API_FUNCTION_LIST_VER;
        f.get_encode_caps = Some(fake_get_caps);
        f.get_encode_preset_config_ex = Some(fake_preset);
        f.initialize_encoder = Some(fake_init);
        f.reconfigure_encoder = Some(fake_reconfigure);
        f.register_async_event = Some(fake_register_event);
        f.unregister_async_event = Some(fake_unregister_event);
        f.create_bitstream_buffer = Some(fake_create_bitstream);
        f.destroy_bitstream_buffer = Some(fake_destroy_bitstream);
        f.encode_picture = Some(fake_encode);
        f.lock_bitstream = Some(fake_lock);
        f.unlock_bitstream = Some(fake_unlock);
        f.destroy_encoder = Some(fake_destroy_encoder);
        Self(f)
    }
}

impl NvTable for FakeTable {
    fn table(&self) -> &nv::NvEncFunctions {
        &self.0
    }
}

enum StageKind {
    HungNvenc {
        core: Core<FakeTable>,
        driver: Box<FakeDriver>,
        lock_seen: Arc<Mutex<u32>>,
    },
    Ready(&'static str),
}

struct StageDouble {
    kind: StageKind,
    size: FrameSize,
}

// SAFETY: test double owned and used on one test thread.
unsafe impl Send for StageDouble {}

impl Drop for StageDouble {
    fn drop(&mut self) {
        if let StageKind::HungNvenc {
            driver, lock_seen, ..
        } = &self.kind
        {
            *lock_seen.lock().unwrap() = driver.lock_calls;
        }
    }
}

impl ScreenEncoder for StageDouble {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let mut bytes = AdmittedBytes::buffer(&frame.grant(), CaptureOwner::CodecOutput, 32)?;
        let keyframe = match &mut self.kind {
            StageKind::HungNvenc { core, .. } => {
                let mut mapped = nv::NvEncMapInputResource::zeroed();
                mapped.mapped_resource = 0x2000usize as nv::Ptr;
                mapped.mapped_buffer_fmt = nv::NV_ENC_BUFFER_FORMAT_NV12;
                core.submit(&mapped, request.frame_id, request.keyframe, &mut |unit| {
                    bytes.extend_from_slice(unit)
                })?
            }
            StageKind::Ready(_) => {
                bytes.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88])?;
                request.keyframe
            }
        };
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::one(bytes)?,
            keyframe,
        })
    }
    fn name(&self) -> String {
        match &self.kind {
            StageKind::HungNvenc { .. } => "nvenc".into(),
            StageKind::Ready(name) => (*name).into(),
        }
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn encode(&mut self, _: &BgraImage, _: bool, _: u64) -> io::Result<EncodedFrame> {
        Err(io::Error::other("admitted only"))
    }
}

struct StageDoubleFactory {
    name: &'static str,
    hung_lock_seen: Option<Arc<Mutex<u32>>>,
}

impl EncoderFactory for StageDoubleFactory {
    fn name(&self) -> String {
        self.name.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::h264(H264Profile::High, true)]
    }
    fn open(
        &self,
        _: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        let kind = match &self.hung_lock_seen {
            Some(lock_seen) => {
                let mut driver = Box::new(FakeDriver::new(true, false));
                let raw: nv::Ptr = (&mut *driver as *mut FakeDriver).cast::<c_void>();
                let core = Core::open(FakeTable::new(), raw, CodecKind::H264, size, fps)?;
                StageKind::HungNvenc {
                    core,
                    driver,
                    lock_seen: lock_seen.clone(),
                }
            }
            None => StageKind::Ready(self.name),
        };
        Ok(Box::new(StageDouble { kind, size }))
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

struct TestOpener(CodecRegistry);

impl EncoderOpener for TestOpener {
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

fn choice(profile: H264Profile) -> CodecChoice {
    CodecChoice {
        codec: CodecKind::H264,
        profile: Some(profile),
        encoder_hardware: true,
        decoder_hardware: true,
        max_size: FrameSize::MAX,
        max_pixels: FrameSize::MAX.pixels(),
        max_macroblocks_per_second: None,
    }
}

#[test]
fn bildkette_task_win_w4_nvenc_async_completes_and_times_out_without_blocking_lock() {
    assert_eq!(size_of::<nv::NvEncEventParams>(), 1544);
    assert_eq!(offset_of!(nv::NvEncEventParams, completion_event), 8);
    assert_eq!(nv::NV_ENC_EVENT_PARAMS_VER, 0x7101_000c);
    assert_eq!(nv::NV_ENC_CAPS_ASYNC_ENCODE_SUPPORT, 30);
    assert_eq!(nv::LOCK_DO_NOT_WAIT, 1);
    assert_eq!(MIN_ASYNC_TIMEOUT_MS, 100);
    assert_eq!(
        [120, 60, 30, 15, 1].map(async_timeout_ms),
        [100, 100, 134, 267, 4_000]
    );

    let size = FrameSize::new(1280, 720);
    let mut no_async = Box::new(FakeDriver::new(false, false));
    let err = Core::open(
        FakeTable::new(),
        (&mut *no_async as *mut FakeDriver).cast::<c_void>(),
        CodecKind::H264,
        size,
        60,
    )
    .err()
    .expect("sync-only driver refused");
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    assert!(no_async.destroyed);

    let mut ok_driver = Box::new(FakeDriver::new(true, true));
    let mut core = Core::open(
        FakeTable::new(),
        (&mut *ok_driver as *mut FakeDriver).cast::<c_void>(),
        CodecKind::H264,
        size,
        60,
    )
    .unwrap();
    core.retune(&RateParams::for_target(12_000_000, 30))
        .unwrap();
    let mut mapped = nv::NvEncMapInputResource::zeroed();
    mapped.mapped_resource = 0x2000usize as nv::Ptr;
    mapped.mapped_buffer_fmt = nv::NV_ENC_BUFFER_FORMAT_NV12;
    let mut out = Vec::new();
    let idr = core
        .submit(&mapped, 1, true, &mut |unit| {
            out.extend_from_slice(unit);
            Ok(())
        })
        .unwrap();
    assert!(idr && out == [0, 0, 0, 1, 0x65, 0x88]);
    drop(core);
    assert_eq!((ok_driver.async_init, ok_driver.async_reinit), (1, 1));
    assert_eq!(
        (ok_driver.lock_calls, ok_driver.last_lock_flags),
        (1, nv::LOCK_DO_NOT_WAIT)
    );
    assert!(ok_driver.bitstream_destroyed && ok_driver.unregistered_event && ok_driver.destroyed);

    let lock_seen = Arc::new(Mutex::new(99u32));
    let registry = CodecRegistry::new(
        vec![
            Arc::new(StageDoubleFactory {
                name: "nvenc",
                hung_lock_seen: Some(lock_seen.clone()),
            }),
            Arc::new(StageDoubleFactory {
                name: "amf",
                hung_lock_seen: None,
            }),
        ],
        Vec::new(),
    );
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "w4-nvenc-timeout",
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    let mut plan = CapturePlan::new(
        SourceKey::new("w4-nvenc-timeout", "src").unwrap(),
        CaptureGeneration::new(1),
    );
    let small = FrameSize::new(64, 64);
    plan.add_bgra(CaptureOwner::SourcePixels, small, 1, CapturePhases::ALL)
        .unwrap();
    plan.add_bytes(CaptureOwner::CodecOutput, 4096, 2, CapturePhases::ALL)
        .unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let img = Picture::Cpu(
        CaptureImageBuilder::new(
            small,
            FrameLimits::LEGACY,
            &grant,
            CaptureOwner::SourcePixels,
        )
        .unwrap()
        .freeze(),
    );
    let req = StageRequest {
        frame_id: 1,
        frame_contract: None,
        visible_rects: None,
        keyframe: true,
        target_bps: 4_000_000,
        size: small,
        choice: choice(H264Profile::High),
        encoder_epoch: 1,
        fps: 60,
    };
    let mut encoding = Encoding::new(Arc::new(TestOpener(registry)));
    let started = Instant::now();
    let (frame, stage_name, _) = encoding
        .encode_with_fallback(&img, Cow::Borrowed(&req), &grant)
        .unwrap();
    let elapsed = started.elapsed();
    assert!(frame.keyframe && stage_name == "amf");
    assert_eq!(encoding.refused, ["nvenc"]);
    assert_eq!(*lock_seen.lock().unwrap(), 0);
    assert!(elapsed.as_millis() >= 90 && elapsed.as_secs() < 3);
}
