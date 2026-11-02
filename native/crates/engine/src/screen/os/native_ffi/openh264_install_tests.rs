//! The OpenH264 pins refuse everything that is not Cisco's file (SCR-M3):
//! no download, no network, no library needed.
use std::io;

use super::*;
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::openh264_pins::CISCO_BINARY_LICENSE;

fn cb_choice() -> CodecChoice {
    CodecChoice {
        max_macroblocks_per_second: None,
        codec: CodecKind::H264,
        profile: Some(H264Profile::ConstrainedBaseline),
        encoder_hardware: false,
        decoder_hardware: false,
        max_size: FrameSize::new(3840, 2160),
        max_pixels: 3840 * 2160,
    }
}

#[test]
fn screen_task_openh264_pin_mismatch_refuses() {
    // The pins: Cisco's HTTPS host, four SHA-256 values, the license word
    // for word.
    assert_eq!(
        hex_sha256(CISCO_BINARY_LICENSE.as_bytes()),
        CISCO_BINARY_LICENSE_SHA256
    );
    assert!(CISCO_BINARY_LICENSE.contains("OpenH264 Video Codec provided by Cisco Systems, Inc."));
    let Some(pin) = PIN else {
        // No Cisco file fits this platform: nothing can be enabled.
        let store = OpenH264Store::at(tempfile::tempdir().unwrap().path().to_path_buf());
        assert!(store
            .enable_from_compressed(CISCO_BINARY_LICENSE_SHA256, b"x")
            .is_err());
        return;
    };
    assert!(pin.url.starts_with(DOWNLOAD_HOST) && DOWNLOAD_HOST.starts_with("https://"));
    for hash in [pin.compressed_sha256, pin.library_sha256] {
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    let dir = tempfile::tempdir().unwrap();
    let store = OpenH264Store::at(dir.path().join("openh264"));
    let library = store.library_path().unwrap();

    // The license must be the one this build shows.
    let refused = store
        .enable_from_compressed("0".repeat(64).as_str(), b"x")
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);

    // A foreign or damaged download, even of the pinned size, is refused
    // and nothing is stored.
    for compressed in [
        b"not the cisco file".to_vec(),
        vec![0u8; pin.compressed_size as usize],
    ] {
        let refused = store
            .enable_from_compressed(CISCO_BINARY_LICENSE_SHA256, &compressed)
            .unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData, "{refused}");
        assert!(refused.to_string().contains("pin"), "{refused}");
        assert!(!library.exists());
        assert_eq!(store.report().state, OpenH264State::Absent);
    }

    // A library file that does not match its pin is never loaded: the
    // report says `failed`, the store hands it out to nobody, and an
    // encoder of the quick inventory fails before loading anything.
    store
        .write_state(true, CISCO_BINARY_LICENSE_SHA256)
        .unwrap();
    crate::support_dirs::write_private_atomic(&library, b"MZ planted library").unwrap();
    let report = store.report();
    assert_eq!(report.state, OpenH264State::Failed);
    assert!(
        report.reason.as_deref().unwrap_or("").contains("pin"),
        "{report:?}"
    );
    let verified = store.verified_library().unwrap_err();
    assert_eq!(verified.kind(), io::ErrorKind::InvalidData);
    let (encoders, _, quick) = crate::screen::os::openh264_codec::codecs(store.clone(), false);
    assert_eq!(quick.state, OpenH264State::Enabled);
    let refused = encoders[0]
        .open(&cb_choice(), FrameSize::new(640, 360), 30)
        .err()
        .expect("no encoder from a planted file");
    assert!(refused.to_string().contains("pin"), "{refused}");
    let (encoders, decoders, checked) =
        crate::screen::os::openh264_codec::codecs(store.clone(), true);
    assert_eq!(checked.state, OpenH264State::Failed);
    assert!(encoders.is_empty() && decoders.is_empty());

    // Switched off, nothing is handed out; removing deletes both files.
    store.disable().unwrap();
    assert_eq!(
        store.verified_library().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    store.remove().unwrap();
    assert!(!library.exists());
    assert_eq!(store.report().state, OpenH264State::Absent);
}
