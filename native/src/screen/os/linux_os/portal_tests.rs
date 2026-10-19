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
use crate::screen::os::spa_pod::{enum_format_pod, parse_video_format, VideoFormat};

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn pad(out: &mut Vec<u8>) {
    while out.len() % 8 != 0 {
        out.push(0);
    }
}

/// An `Id` pod: size 4, type 3, the id, 4 bytes of padding.
fn id_pod(out: &mut Vec<u8>, id: u32) {
    word(out, 4);
    word(out, 3);
    word(out, id);
    word(out, 0);
}

/// A choice pod over `values` of `child_size` bytes and `child_type`.
fn choice_pod(out: &mut Vec<u8>, kind: u32, child_size: u32, child_type: u32, values: &[u32]) {
    let body_size = 8 + 8 + values.len() as u32 * 4;
    word(out, body_size);
    word(out, 19);
    word(out, kind);
    word(out, 0);
    word(out, child_size);
    word(out, child_type);
    for value in values {
        word(out, *value);
    }
    pad(out);
}

#[test]
fn screen_task_spa_pod_enum_format_bytes() {
    // Written out from the description of `spa/param/video/format-utils.h`
    // and `spa/pod/pod.h`: object, then key/flags/pod for each property.
    let mut body = Vec::new();
    word(&mut body, 0x40003); // SPA_TYPE_OBJECT_Format
    word(&mut body, 3); // SPA_PARAM_EnumFormat
    word(&mut body, 1); // mediaType
    word(&mut body, 0);
    id_pod(&mut body, 2); // video
    word(&mut body, 2); // mediaSubtype
    word(&mut body, 0);
    id_pod(&mut body, 1); // raw
    word(&mut body, 0x20001); // VIDEO_format
    word(&mut body, 0);
    choice_pod(&mut body, 3, 4, 3, &[8, 8, 7, 12, 11]); // Enum of Ids
    word(&mut body, 0x20003); // VIDEO_size
    word(&mut body, 0);
    choice_pod(&mut body, 1, 8, 10, &[1920, 1080, 1, 1, 16384, 16384]); // Range
    word(&mut body, 0x20004); // VIDEO_framerate
    word(&mut body, 0);
    choice_pod(&mut body, 1, 8, 11, &[0, 1, 0, 1, 1000, 1]);
    let mut expected = Vec::new();
    word(&mut expected, body.len() as u32);
    word(&mut expected, 15); // Object
    expected.extend_from_slice(&body);
    pad(&mut expected);
    let built = enum_format_pod();
    assert_eq!(built, expected);
    let size = u32::from_le_bytes(built[0..4].try_into().unwrap()) as usize;
    assert_eq!(size, built.len() - 8);
    assert_eq!(built.len() % 8, 0);
    // No VIDEO_modifier (key 0x20005): producers keep to shared memory.
    assert!(!built
        .windows(4)
        .any(|word| word == 0x20005u32.to_le_bytes()));
}

#[test]
fn spa_pod_negotiated_format_is_read_and_garbage_is_not() {
    let mut body = Vec::new();
    word(&mut body, 0x40003);
    word(&mut body, 4); // SPA_PARAM_Format
    word(&mut body, 1);
    word(&mut body, 0);
    id_pod(&mut body, 2);
    word(&mut body, 0x20001);
    word(&mut body, 0);
    id_pod(&mut body, 12); // BGRA
    word(&mut body, 0x20003);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 10); // Rectangle
    word(&mut body, 1920);
    word(&mut body, 1080);
    word(&mut body, 0x20004);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 11); // Fraction
    word(&mut body, 60);
    word(&mut body, 1);
    let mut pod = Vec::new();
    word(&mut pod, body.len() as u32);
    word(&mut pod, 15);
    pod.extend_from_slice(&body);
    assert_eq!(
        parse_video_format(&pod),
        Some((VideoFormat::Bgra, FrameSize::new(1920, 1080)))
    );
    // The offer holds choices, not values: nothing negotiated.
    assert_eq!(parse_video_format(&enum_format_pod()), None);
    assert_eq!(parse_video_format(&pod[..pod.len() - 5]), None);
    assert_eq!(parse_video_format(&[0u8; 3]), None);
    assert_eq!(parse_video_format(&[]), None);
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
    latest.publish(picture(640, 360));
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
    latest.publish(picture(800, 600));
    assert!(matches!(
        source.capture(5, false).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(source.size(), FrameSize::new(800, 600));
    latest.fail("pipewire_stream: gone".into());
    assert!(source.capture(6, false).is_err());
}

#[test]
fn portal_for_remote_desktop_waits_for_u22() {
    let options = PortalOptions {
        remote_desktop: true,
        ..PortalOptions::default()
    };
    let error = PortalSession::start(options).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
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

fn first_frame(source: &mut PortalSource, within: Duration) -> BgraImage {
    let started = Instant::now();
    loop {
        match source.capture(1, true).expect("the portal source works") {
            SourceFrame::Frame(image) => return image,
            SourceFrame::Paused(_) | SourceFrame::Unchanged => {}
        }
        assert!(started.elapsed() < within, "no picture from the portal");
        std::thread::sleep(Duration::from_millis(50));
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
            encoder.encode(&image, true, 0).unwrap();
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
