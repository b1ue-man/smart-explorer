//! Real X11 resize with productive persistent Tile/Hybrid encoder owners.
//! The final remote suite supplies an isolated DISPLAY; no local entrypoint.
use super::*;
use crate::screen::core::capture_plan::bgra_bytes;
use crate::screen::core::frame_budget::{FrameContract, FRAME_PROTOCOL};
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::host_codec_adapters::{AdmittedTilesEncoder, HostCodecPlan, HostContract};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::negotiate::{CodecChoice, CodecKind};
use x11rb::protocol::xproto::{ConfigureWindowAux, CreateWindowAux, WindowClass};

struct WindowOwner {
    connection: RustConnection,
    window: Window,
}
impl Drop for WindowOwner {
    fn drop(&mut self) {
        let _ = self.connection.destroy_window(self.window);
        let _ = self.connection.flush();
    }
}
fn fixture_window() -> (WindowOwner, FrameSize) {
    assert_eq!(
        std::env::var("SE_SCREEN_TASK_X11").as_deref(),
        Ok("1"),
        "the remote suite must provide its isolated X11 fixture"
    );
    let (connection, screen) = RustConnection::connect(None).expect("isolated X11 DISPLAY");
    let setup = &connection.setup().roots[screen];
    let size = FrameSize::new(
        u32::from(setup.width_in_pixels).min(1024),
        u32::from(setup.height_in_pixels).min(768),
    );
    assert!(
        size.width >= 640 && size.height >= 480,
        "isolated X11 root must be at least 640 x 480"
    );
    let root = setup.root;
    let window = connection.generate_id().unwrap();
    connection
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            64,
            64,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(0x0012_3456)
                .override_redirect(1),
        )
        .unwrap()
        .check()
        .unwrap();
    connection.map_window(window).unwrap().check().unwrap();
    connection.get_input_focus().unwrap().reply().unwrap();
    (WindowOwner { connection, window }, size)
}
fn request(contract: Option<FrameContract>) -> StageRequest {
    let size = FrameSize::new(64, 64);
    StageRequest {
        frame_id: 1,
        frame_contract: contract,
        visible_rects: None,
        keyframe: true,
        target_bps: 2_000_000,
        size,
        encoder_epoch: 1,
        fps: 10,
        choice: CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: size,
            max_pixels: size.pixels(),
            max_macroblocks_per_second: None,
        },
    }
}
fn capture_image(source: &mut X11Source, frame: u64) -> CapturedImage {
    match source.capture(frame, true).unwrap() {
        SourceFrame::Frame(image) => image,
        _ => panic!("the visible X11 fixture must yield an image"),
    }
}
fn resize_with_encoder(hybrid: bool, shm: bool) {
    let (window, new_size) = fixture_window();
    let old_size = FrameSize::new(64, 64);
    let key = SourceKey::new(
        "x11-resize-retry",
        &format!("linux:window:{}", window.window),
    )
    .unwrap();
    let contract = hybrid.then_some(FrameContract {
        version: FRAME_PROTOCOL,
        limits: FrameLimits {
            max_size: old_size,
            max_pixels: old_size.pixels(),
            working_bytes: 2 * 1024 * 1024,
        },
        hybrid_regions: true,
    });
    let output = HostOutputPlan::new(
        contract.map_or(HostContract::Legacy, HostContract::Versioned),
        old_size,
        HostCodecPlan::Tiles,
    )
    .unwrap()
    .for_worker();
    let next = capture_memory::plan(
        &key,
        CaptureGeneration::new(2),
        new_size,
        shm,
        0,
        Some(&output),
    )
    .unwrap();
    let capacity = next.peak_bytes();
    if let Some(contract) = contract {
        assert!(capacity >= contract.limits.working_bytes);
    }
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(capacity),
            capacity,
            key.session(),
        )
        .unwrap();
    admission.bind_contract(contract).unwrap();
    let mut source = X11Source::prepare(
        &SourceId::Window(window.window.to_string()),
        key.clone(),
        admission.clone(),
        false,
    )
    .ok()
    .unwrap();
    if shm {
        assert!(source.shm, "the isolated X11 fixture must support MIT-SHM");
    }
    source.shm = shm;
    source.cursor = false;
    // The window is unobscured and override-redirect. Exercise both actual
    // SHM and GetImage producers without requiring a compositor on the runner.
    source.compositor = false;
    let grant = admission
        .authority()
        .approve(source.plan(Some(&output)).unwrap())
        .unwrap();
    let mut source = source.activate(grant.clone(), Some(output)).ok().unwrap();
    let first = capture_image(&mut source, 1);
    let request = request(contract);
    let tiles: Box<dyn ScreenEncoder> = Box::new(
        AdmittedTilesEncoder::new(old_size, FrameLimits::RESOURCE, grant.clone()).unwrap(),
    );
    let mut encoder =
        crate::screen::core::host_hybrid::wrap_captured(tiles, &request, grant.clone()).unwrap();
    let encoded = encoder.encode_captured(&first, &request).unwrap();
    assert!(!encoded.records.is_empty());
    drop(encoded);
    drop(first);
    let before = source.reference_generation();
    window
        .connection
        .configure_window(
            window.window,
            &ConfigureWindowAux::new()
                .width(new_size.width)
                .height(new_size.height),
        )
        .unwrap()
        .check()
        .unwrap();
    window
        .connection
        .get_input_focus()
        .unwrap()
        .reply()
        .unwrap();
    let denied = source
        .capture(2, false)
        .err()
        .expect("old persistent encoder ownership must delay the exact new plan");
    assert_eq!(denied.kind(), io::ErrorKind::WouldBlock);
    assert!(
        source.reference_generation() > before,
        "the worker must see invalidation on the failed capture"
    );
    let pending_reference = source.reference_generation();
    let pending_generation = source.generation;
    assert_eq!(source.size(), new_size);
    assert!(source.grant.is_none() && source.last_image.is_none() && source.segment.is_none());
    assert!(grant.is_retired().unwrap());
    assert!(
        grant.live_bytes().unwrap() >= bgra_bytes(old_size).unwrap() * if hybrid { 2 } else { 1 }
    );
    let backend = super::super::host_backend::LinuxBackend::new();
    assert!(source.inject_input(&backend, before, true, &[]).is_err());
    assert!(
        source
            .inject_input(&backend, pending_reference, true, &[])
            .is_err(),
        "new reference without pixels cannot accept input"
    );
    let denied = source
        .capture(3, true)
        .err()
        .expect("keyframe does not waive live owners");
    assert_eq!(denied.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(source.reference_generation(), pending_reference);
    assert_eq!(source.generation, pending_generation);
    // This is the real owner's drop that Worker::produce/prepare_reply performs
    // after observing the changed source reference. No accounting is forged.
    drop(encoder);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    let current = capture_image(&mut source, 4);
    assert_eq!(current.size(), new_size);
    assert_eq!(current.source(), &key);
    assert_eq!(current.generation(), pending_generation);
    assert_eq!(source.reference_generation(), pending_reference);
    assert_eq!(&current.image().data()[..4], &[0x56, 0x34, 0x12, 0xff]);
    assert!(source.grant.is_some());
    assert!(matches!(
        source.capture(5, false).unwrap(),
        SourceFrame::Unchanged
    ));
}
#[test]
#[ignore = "requires isolated X11 DISPLAY >=640x480, MIT-SHM and SE_SCREEN_TASK_X11=1"]
fn batches_task_b6_x11_tiles_resize_releases_old_reference_before_retry() {
    for shm in [false, true] {
        resize_with_encoder(false, shm);
    }
}
#[test]
#[ignore = "requires isolated X11 DISPLAY >=640x480, MIT-SHM and SE_SCREEN_TASK_X11=1"]
fn batches_task_b6_x11_hybrid_resize_releases_old_reference_before_retry() {
    for shm in [false, true] {
        resize_with_encoder(true, shm);
    }
}
