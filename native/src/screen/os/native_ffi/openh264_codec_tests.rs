//! OpenH264 with Cisco's library (SCR-M3). The suite downloads the pinned
//! file in its job and names it in `SE_SCREEN_TASK_OPENH264` (the `.bz2`);
//! the tests install it into a temporary store through the pin checks of
//! production. `screen_task_openh264_decodes_constrained_high` also needs
//! `SE_SCREEN_TASK_H264_HIGH`: an Annex B file of 30 frames 640 × 360, High
//! profile without B frames, made on the runner by
//! `ffmpeg -f lavfi -i testsrc=size=640x360:rate=30 -frames:v 30 -c:v libx264
//! -profile:v high -bf 0 -pix_fmt yuv420p -f h264 high.h264` (x264 is only a
//! test tool). The three tests share `TEST_LOCK`, because one of them looks
//! at the libraries this process has loaded.
use std::sync::{Arc, MutexGuard, PoisonError};

use super::*;
use crate::screen::core::bench::{run_local, BenchOptions};
use crate::screen::core::codec_registry::CodecRegistry;
use crate::screen::core::negotiate::{CodecKind, CodecPreference};
use crate::screen::os::openh264_pins::{CISCO_BINARY_LICENSE_SHA256, PIN};

pub(crate) const OPENH264_ENV: &str = "SE_SCREEN_TASK_OPENH264";
const HIGH_ENV: &str = "SE_SCREEN_TASK_H264_HIGH";

fn locked() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A temporary store with Cisco's library from `SE_SCREEN_TASK_OPENH264`.
pub(crate) fn store_with_openh264() -> (tempfile::TempDir, OpenH264Store) {
    let path = std::env::var(OPENH264_ENV)
        .unwrap_or_else(|_| panic!("{OPENH264_ENV} names the pinned OpenH264 .bz2"));
    let compressed = std::fs::read(&path).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = OpenH264Store::at(dir.path().join("openh264"));
    store
        .enable_from_compressed(CISCO_BINARY_LICENSE_SHA256, &compressed)
        .unwrap();
    (dir, store)
}

/// Only the OpenH264 coders of `store` and the tiles.
pub(crate) fn openh264_registry(store: &OpenH264Store) -> CodecRegistry {
    let (encoders, decoders, report) = codecs(store.clone(), true);
    assert_eq!(report.state, OpenH264State::Enabled, "{report:?}");
    CodecRegistry::new(encoders, decoders)
}

/// Whether this process has mapped the OpenH264 library.
#[cfg(target_os = "linux")]
fn library_loaded() -> bool {
    PIN.is_some_and(|pin| {
        std::fs::read_to_string("/proc/self/maps")
            .unwrap()
            .contains(pin.library_name)
    })
}

#[cfg(windows)]
fn library_loaded() -> bool {
    PIN.is_some_and(|pin| {
        let name: Vec<u16> = pin.library_name.encode_utf16().chain([0]).collect();
        // SAFETY: a terminated wide name; the handle is not kept.
        let module =
            unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(name.as_ptr()) };
        !module.is_null()
    })
}

#[test]
#[ignore = "needs SE_SCREEN_TASK_OPENH264 (the suite downloads the pinned file)"]
fn screen_task_local_bench_h264_openh264() {
    let _lock = locked();
    let (_dir, store) = store_with_openh264();
    let codecs = openh264_registry(&store);
    let options = BenchOptions {
        codec: CodecPreference::H264,
        size: FrameSize::new(1280, 720),
        fps: 30,
        seconds: 3,
    };
    options.decoders(&codecs).unwrap();
    let report = run_local(options, &codecs).unwrap();
    assert_eq!(report.codec, "h264", "{report:?}");
    assert_eq!(report.encoder, OPENH264_NAME, "{report:?}");
    assert_eq!(report.decoder, OPENH264_NAME, "{report:?}");
    assert_eq!(report.path, "local");
    assert_eq!(report.corrupt_frames, 0, "{report:?}");
    // Thresholds of C9 (assumption A1) for 1280 × 720 at 30 frames/s.
    assert!(report.fps_mean >= 27.0, "{report:?}");
    assert!(report.latency_ms.p95 <= 150.0, "{report:?}");
    assert!(
        report.static_psnr_db.is_none_or(|db| db >= 38.0),
        "{report:?}"
    );
    assert!(report.keyframes >= 1, "{report:?}");
}

#[test]
#[ignore = "needs SE_SCREEN_TASK_OPENH264 (the suite downloads the pinned file)"]
fn screen_task_openh264_disable_unloads() {
    let _lock = locked();
    let (_dir, store) = store_with_openh264();
    let codecs = openh264_registry(&store);
    let choice = crate::screen::core::negotiate::negotiate(
        &codecs.encoder_offers(),
        &codecs.decoder_offers(),
        CodecPreference::H264,
    )[0];
    assert_eq!(choice.codec, CodecKind::H264);
    let size = FrameSize::new(640, 360);
    let mut opened = codecs.open_encoder(&choice, size, 30).unwrap();
    let image = BgraImage::new(size).unwrap();
    let frame = opened.encoder.encode(&image, true, 1_000_000).unwrap();
    assert!(frame.keyframe && !frame.records.is_empty());
    assert!(library_loaded(), "the encoder loaded OpenH264");

    // Switched off: no coder is offered or opened any more, and once the
    // running encoder goes, the library is no longer in this process.
    store.disable().unwrap();
    let (encoders, decoders, report) = codecs_of(&store);
    assert_eq!(report.state, OpenH264State::Disabled);
    assert!(encoders.is_empty() && decoders.is_empty());
    let refused = codecs
        .open_encoder(&choice, size, 30)
        .err()
        .expect("switched off");
    assert!(refused.to_string().contains("switched off"), "{refused}");
    drop(opened);
    assert!(
        !library_loaded(),
        "OpenH264 stayed loaded after switching it off"
    );

    // Switching on again needs no download.
    store.enable(CISCO_BINARY_LICENSE_SHA256).unwrap();
    assert_eq!(store.report().state, OpenH264State::Enabled);
}

fn codecs_of(
    store: &OpenH264Store,
) -> (
    Vec<Arc<dyn EncoderFactory>>,
    Vec<Arc<dyn DecoderFactory>>,
    OpenH264Report,
) {
    codecs(store.clone(), false)
}

#[test]
#[ignore = "needs SE_SCREEN_TASK_OPENH264 and SE_SCREEN_TASK_H264_HIGH (made by the suite)"]
fn screen_task_openh264_decodes_constrained_high() {
    let _lock = locked();
    let (_dir, store) = store_with_openh264();
    let path =
        std::env::var(HIGH_ENV).unwrap_or_else(|_| panic!("{HIGH_ENV} names the High test stream"));
    let stream = std::fs::read(path).unwrap();
    // The stream is High (profile_idc 100) without B frames (x264 -bf 0).
    let units: Vec<&[u8]> = openh264::nal_units(&stream).collect();
    let sps = units
        .iter()
        .find_map(|unit| {
            let start = unit.iter().position(|byte| *byte == 1)? + 1;
            (unit.get(start)? & 0x1f == 7).then(|| unit.get(start + 1).copied())?
        })
        .expect("a sequence parameter set");
    assert_eq!(sps, 100, "profile_idc of the test stream");
    let size = FrameSize::new(640, 360);
    let header = crate::screen::core::wire_binary::GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 1,
        kind: crate::screen::core::wire_binary::StreamKind::Video,
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
        size,
    };
    let decoders = codecs_of(&store).1;
    let mut decoder = decoders[0].open(&header).unwrap();
    let mut pictures = 0;
    for (index, unit) in units.iter().enumerate() {
        let record = FrameRecord {
            frame_id: index as u64 + 1,
            capture_ts_us: 0,
            flags: crate::screen::core::wire_binary::FLAG_KEYFRAME,
            payload: unit.to_vec(),
        };
        if decoder.decode(&record).unwrap() {
            pictures += 1;
            let data = decoder.picture().data();
            assert!(data.chunks_exact(4).any(|pixel| pixel[..3] != data[..3]));
        }
    }
    assert!(
        pictures >= 25,
        "{pictures} of 30 pictures of the High stream"
    );
}
