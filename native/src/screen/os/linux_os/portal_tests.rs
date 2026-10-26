//! Acceptance of the portal path (SCR-M5): the SPA pod bytes, the picture
//! conversion, the mapping of the portal states to the capture, and the two
//! stages with a real Wayland session (`linux-wayland`, headless Sway with
//! `xdg-desktop-portal-wlr` and PipeWire, `SE_SCREEN_TASK_WAYLAND=1`).
use std::time::{Duration, Instant};

use super::*;
use crate::screen::core::host_backend::ScreenEncoder;
use crate::screen::core::host_backend::TilesScreenEncoder;
use crate::screen::core::pixel_convert::BgraImage;
use crate::screen::os::pipewire_stream::convert_rows;
use crate::screen::os::spa_pod::VideoFormat;

fn admitted_picture(image:BgraImage)->crate::screen::core::capture_image::CapturedImage {
    use crate::screen::core::capture_admission::{SourceKey,CaptureGeneration,CaptureOwner};
    use crate::screen::core::capture_plan::{CapturePlan,CapturePhases};
    let admission=super::super::host_backend::prepared::test_admission("portal-pixels");
    let mut plan=CapturePlan::new(SourceKey::new("portal-pixels","portal").unwrap(),CaptureGeneration::new(1));
    plan.add_bgra(CaptureOwner::SourcePixels,image.size(),1,CapturePhases::ALL).unwrap();
    let grant=admission.authority().approve(plan).unwrap();
    let mut builder=crate::screen::core::capture_image::CaptureImageBuilder::new(image.size(),
        crate::screen::core::frame_budget::FrameLimits::RESOURCE,&grant,CaptureOwner::SourcePixels).unwrap();
    builder.pixels_mut().copy_from_slice(image.data());builder.freeze()
}

#[test]
fn pipewire_rows_become_opaque_bgra() {
    let size = FrameSize::new(2, 2);
    // Rows of 12 bytes (stride 12): 8 used, 4 padding.
    let mut src = vec![0u8; 12 * 2];
    src[0..8].copy_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
    src[12..20].copy_from_slice(&[7, 8, 9, 0, 10, 11, 12, 0]);
    let bgrx = convert_rows(&src, 12, VideoFormat::Bgrx, size).unwrap();
    assert_eq!(
        bgrx.data(),
        &[1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
    );
    let rgbx = convert_rows(&src, 12, VideoFormat::Rgbx, size).unwrap();
    assert_eq!(&rgbx.data()[..8], &[3, 2, 1, 255, 6, 5, 4, 255]);
    // A buffer shorter than its picture is an error, never a read past it.
    assert!(convert_rows(&src[..19], 12, VideoFormat::Bgra, size).is_err());
    assert!(convert_rows(&src, 7, VideoFormat::Bgra, size).is_err());
}

fn picture(width: u32, height: u32) -> BgraImage {
    BgraImage::new(FrameSize::new(width, height)).unwrap()
}

#[test]
fn portal_source_maps_the_portal_states() {
    let paused = |state| matches!(state, SourceFrame::Paused(_));
    let mut waiting = PortalSource::with_handle(PortalHandle::detached(PortalState::Waiting));
    assert_eq!(waiting.size(), PORTAL_PROVISIONAL);
    match waiting.capture(1, true).unwrap() {
        SourceFrame::Paused(state) => assert_eq!(state, SessionState::WaitingForPortal),
        _ => panic!("not paused"),
    }
    let denied = PortalHandle::detached(PortalState::Denied("cancelled".into()));
    let error = PortalSource::with_handle(denied)
        .capture(1, true)
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "portal_denied: cancelled");
    let missing = PortalHandle::detached(PortalState::Failed(PIPEWIRE_MISSING.into()));
    match PortalSource::with_handle(missing).capture(1, true).unwrap() {
        SourceFrame::Paused(state) => assert_eq!(state, SessionState::NoDisplay),
        _ => panic!("not paused"),
    }
    let broken = PortalHandle::detached(PortalState::Failed("portal: gone".into()));
    let error = PortalSource::with_handle(broken)
        .capture(1, true)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "portal: gone");
    // Streaming: waiting until the first picture, then the real size.
    let latest = Arc::new(LatestFrame::default());
    let mut source = PortalSource::with_handle(PortalHandle::detached(PortalState::Streaming(
        latest.clone(),
    )));
    assert!(paused(source.capture(1, false).unwrap()));
    latest.publish(admitted_picture(picture(640, 360)));
    assert!(matches!(
        source.capture(2, false).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(source.size(), FrameSize::new(640, 360));
    assert!(matches!(
        source.capture(3, false).unwrap(),
        SourceFrame::Unchanged
    ));
    // A key frame always has a picture.
    assert!(matches!(
        source.capture(4, true).unwrap(),
        SourceFrame::Frame(_)
    ));
    latest.publish(admitted_picture(picture(800, 600)));
    assert!(matches!(
        source.capture(5, false).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(source.size(), FrameSize::new(800, 600));
    latest.fail("pipewire_stream: gone".into());
    assert!(source.capture(6, false).is_err());
}

/// A session that may control without PipeWire fails before any dialog
/// (nothing to cast), and input stays refused (SCR-M6).
#[test]
fn screen_task_wayland_remote_session_without_pipewire_asks_nothing() {
    let options = PortalOptions {
        remote_desktop: true,
        pipewire_library: "libpipewire-0.3-missing-for-test.so",
    };
    let handle = PortalSession::start(options).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if matches!(&*handle.lock(), PortalState::Failed(text) if text == PIPEWIRE_MISSING) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no pipewire_missing");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let text = [crate::screen::core::input_model::InputEvent::Text { text: "x".into() }];
    assert_eq!(
        super::super::portal_input::inject(&text),
        Err(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)
    );
}

fn wayland_stage() -> bool {
    std::env::var("SE_SCREEN_TASK_WAYLAND").is_ok_and(|value| value == "1")
}

/// Points the app data of this process (and so the token file) to a new
/// directory.
fn private_data_home() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", directory.path());
    directory
}

/// Why the wait for the first picture ended without one.
enum NoPicture {
    /// `capture` failed.
    Source(io::Error),
    /// The time ran out; the text names where the session stands.
    Silent(String),
}

/// Waits for the first picture; `nudge` runs every 250 ms meanwhile. A
/// compositor records a picture when the screen changes, so a check that can
/// change the screen does so here; the picture still has to arrive.
fn await_picture(
    source: &mut PortalSource,
    within: Duration,
    mut nudge: impl FnMut(),
) -> Result<BgraImage, NoPicture> {
    let started = Instant::now();
    let mut next_nudge = started;
    loop {
        match source.capture(1, true).map_err(NoPicture::Source)? {
            SourceFrame::Frame(image) => return Ok(image.image().clone()),
            SourceFrame::Paused(_) | SourceFrame::Unchanged => {}
        }
        if started.elapsed() >= within {
            return Err(NoPicture::Silent(source.describe()));
        }
        if Instant::now() >= next_nudge {
            nudge();
            next_nudge = Instant::now() + Duration::from_millis(250);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A DRM render node (`/dev/dri/renderD*`) exists on this machine.
fn has_render_node() -> bool {
    std::fs::read_dir("/dev/dri").is_ok_and(|mut entries| {
        entries.any(|entry| {
            entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
        })
    })
}

/// The first picture of a wlroots portal; the check ends as "nicht
/// ausgeführt" where that portal cannot cast at all. xdg-desktop-portal-wlr
/// 0.7.x (Ubuntu 24.04: 0.7.1) answers `Start` with an error (`start_screencast`
/// returns -1, "unable to receive a valid format from wlr_screencopy", logged
/// at INFO only) unless wlr-screencopy offers a DMA-BUF format, and wlroots
/// 0.17 offers one only for an output with a DMA-BUF allocator (a GBM device on
/// a render node, `wlr_screencopy_v1.c`: `output->allocator->buffer_caps`).
/// xdg-desktop-portal then reports response 2 ("Other") and "A backend call
/// failed: Operation not permitted" (-EPERM is the -1).
fn first_frame(source: &mut PortalSource, within: Duration) -> BgraImage {
    match await_picture(source, within, || {}) {
        Ok(image) => image,
        Err(NoPicture::Source(error))
            if error.kind() == io::ErrorKind::PermissionDenied
                && error.to_string() == "portal_denied: Other"
                && !has_render_node() =>
        {
            panic!(
                "nicht ausgeführt: xdg-desktop-portal-wlr 0.7 startet ohne DMA-BUF-Format von \
                 wlr-screencopy keinen Screencast, und der kopflose Sway dieses Rechners hat \
                 ohne DRM-Renderknoten (/dev/dri/renderD*) keinen DMA-BUF-Allocator ({error})"
            )
        }
        Err(NoPicture::Source(error)) => panic!("the portal source works: {error:?}"),
        Err(NoPicture::Silent(state)) => panic!("no picture from the portal; {state}"),
    }
}

/// Waits until the stream of a session shows that it runs; `nudge` runs
/// every 250 ms and moves the pointer. A picture shows it, and so do chunks
/// that carry only the cursor: KWin 5.27 emits `Output::outputChange`, the
/// only trigger of a frame with content (`ScreencastManager::streamOutput`),
/// in its DRM and nested Wayland backends and not in `--virtual`, and
/// `xdg-desktop-portal-kde` starts the streams of a RemoteDesktop session
/// with cursor mode Metadata whatever the program asked for
/// (`RemoteDesktopPortal::Start`), so a pointer move reaches the stream as a
/// chunk of size 0 flagged corrupted (`recordCursor`). `--virtual` has no
/// pointer of its own: those chunks follow the moves `nudge` injects. The
/// answer names what arrived.
fn stream_runs_nudged(source: &mut PortalSource, within: Duration, nudge: impl FnMut()) -> String {
    match await_picture(source, within, nudge) {
        Ok(image) => format!("picture {:?}", image.size()),
        Err(NoPicture::Source(error)) => panic!("the portal source works: {error:?}"),
        Err(NoPicture::Silent(state)) if source.only_empty_chunks() => {
            format!("cursor chunks only; {state}")
        }
        Err(NoPicture::Silent(state)) => panic!("no picture from the portal; {state}"),
    }
}

#[test]
#[ignore = "needs headless Sway, xdg-desktop-portal-wlr and PipeWire (SE_SCREEN_TASK_WAYLAND=1)"]
fn screen_task_wayland_portal_bench() {
    assert!(wayland_stage(), "SE_SCREEN_TASK_WAYLAND=1 is not set");
    let _data = private_data_home();
    let mut source = PortalSource::open(PortalOptions::default()).unwrap();
    // The dialog (here: none, `chooser_type = none`) has no time limit of its
    // own; the test waits this long for a picture.
    let first = first_frame(&mut source, Duration::from_secs(120));
    assert!(first.size().is_valid_frame());
    assert_eq!(source.size(), first.size());
    // Capture and tile coding of real pictures, key frames only.
    let mut encoder = TilesScreenEncoder::new(first.size()).unwrap();
    let started = Instant::now();
    let mut frames = 0u32;
    while started.elapsed() < Duration::from_secs(3) {
        if let SourceFrame::Frame(image) = source.capture(0, true).unwrap() {
            encoder.encode(image.image(), true, 0).unwrap();
            frames += 1;
        }
    }
    let rate = f64::from(frames) / started.elapsed().as_secs_f64();
    println!(
        "wayland portal capture+tiles: {rate:.1} pictures/s at {:?}",
        first.size()
    );
    // Threshold of assumption A1 until the first remote measurement.
    assert!(rate >= 10.0, "{rate:.1} pictures/s");
    // The grant is stored for the next start.
    assert!(portal_tokens::load().get(TokenKind::ScreenCast).is_some());
}

#[test]
#[ignore = "needs headless Sway, xdg-desktop-portal-wlr with XDPW_PERSIST_MODE=permanent (SE_SCREEN_TASK_WAYLAND=1)"]
fn screen_task_wayland_token_replaced_after_start() {
    assert!(wayland_stage(), "SE_SCREEN_TASK_WAYLAND=1 is not set");
    let _data = private_data_home();
    assert!(portal_tokens::load().get(TokenKind::ScreenCast).is_none());
    let mut first = PortalSource::open(PortalOptions::default()).unwrap();
    first_frame(&mut first, Duration::from_secs(120));
    let stored = portal_tokens::load();
    let one = stored
        .get(TokenKind::ScreenCast)
        .expect("token after the first start")
        .clone();
    drop(first);
    // The second start hands the stored token to the portal and stores the
    // token the portal answers with, replacing the old one (A10: this works
    // without registering the program with the portal).
    std::thread::sleep(Duration::from_millis(1100));
    let mut second = PortalSource::open(PortalOptions::default()).unwrap();
    first_frame(&mut second, Duration::from_secs(120));
    let stored = portal_tokens::load();
    let two = stored
        .get(TokenKind::ScreenCast)
        .expect("token after the second start");
    assert!(
        two.saved_at > one.saved_at,
        "the token was not stored again"
    );
    println!(
        "portal token changed by the second start: {}",
        two.token != one.token
    );
}

/// KWin without a display, its portal and a permission store that allows
/// remote control (`SE_SCREEN_TASK_KDE=1`, K3): the session that may control
/// streams and takes a click and text; without that runner it is not run.
#[test]
#[ignore = "needs headless KWin, xdg-desktop-portal-kde, a permission store entry and PipeWire (SE_SCREEN_TASK_KDE=1)"]
fn screen_task_kde_remote_desktop_input() {
    if std::env::var("SE_SCREEN_TASK_KDE").ok().as_deref() != Some("1") {
        eprintln!("not run: no headless KWin with RemoteDesktop permission");
        return;
    }
    use crate::screen::core::input_model::{InputEvent, PointerButton};
    let _home = private_data_home();
    let options = PortalOptions {
        remote_desktop: true,
        ..PortalOptions::default()
    };
    let mut source = PortalSource::with_handle(PortalSession::start(options).unwrap());
    // The pointer moves while the stream is awaited: with the cursor in the
    // picture (`CursorMode::Embedded`) that is a change of the screen, with
    // cursor metadata (KDE's RemoteDesktop streams) a cursor chunk; the
    // session only takes input once the stream runs.
    let mut step = 0u32;
    let answered = stream_runs_nudged(&mut source, Duration::from_secs(60), || {
        step += 1;
        let at: u16 = 16_384 + (step % 2) as u16 * 32_768;
        let _ = super::super::portal_input::inject(&[InputEvent::PointerAbs { x: at, y: at }]);
    });
    println!("KDE RemoteDesktop stream: {answered}");
    let events = [
        InputEvent::PointerAbs {
            x: 32_768,
            y: 32_768,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: true,
        },
        InputEvent::Button {
            button: PointerButton::Left,
            down: false,
        },
        InputEvent::Text { text: "se".into() },
    ];
    assert_eq!(super::super::portal_input::inject(&events), Ok(()));
}

impl PortalSource {
    /// Where the session stands, for the failure text of a source that
    /// shows no picture: the portal still waiting (dialog, portal service),
    /// a failure, or the stream and what it did.
    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        match &*self.handle.lock() {
            PortalState::Waiting | PortalState::WaitingInactive => {
                "portal: no answer to Start yet (dialog or portal service)".into()
            }
            PortalState::Streaming(latest) => format!("portal: streaming; {}", latest.describe()),
            PortalState::Denied(text) => format!("portal: denied: {text}"),
            PortalState::Failed(text) => format!("portal: failed: {text}"),
        }
    }
}

#[cfg(test)]
impl PortalSource {
    /// The stream runs and has sent only empty or corrupted chunks so far:
    /// the compositor reports cursor moves, no change of the screen.
    pub(crate) fn only_empty_chunks(&self) -> bool {
        match &*self.handle.lock() {
            PortalState::Streaming(latest) => latest.only_empty_chunks(),
            _ => false,
        }
    }
}
