//! The OpenH264 pins refuse everything that is not Cisco's file (SCR-M3),
//! and the automatic installation retries on its schedule and respects the
//! user's switch: no download, no network, no library needed.
use std::cell::Cell;
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

    // Switched off, nothing is handed out; removing deletes the library and
    // keeps OpenH264 switched off, so nothing fetches it again.
    store.disable().unwrap();
    assert_eq!(
        store.verified_library().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    store.remove().unwrap();
    assert!(!library.exists());
    assert_eq!(store.report().state, OpenH264State::Disabled);
    assert!(!store.report().library_present);
}

/// A fetch that fails like a missing network and counts its calls.
fn offline(calls: &Cell<u32>) -> impl Fn(&OpenH264Pin) -> io::Result<Vec<u8>> + '_ {
    move |_| {
        calls.set(calls.get() + 1);
        Err(io::Error::new(io::ErrorKind::NotConnected, "no answer"))
    }
}

#[test]
fn screen_task_openh264_auto_install_retries_and_respects_the_switch() {
    let dir = tempfile::tempdir().unwrap();
    let store = OpenH264Store::auto_at(dir.path().join("openh264"));
    assert!(!OpenH264Store::at(dir.path().join("x")).installs_on_demand());
    if PIN.is_none() {
        // Nothing is offered here: nothing installs on demand.
        assert!(!store.installs_on_demand());
        assert!(!store.session_ready(|| false, |_| panic!("no pin, no download")));
        return;
    }
    assert!(store.installs_on_demand());
    let calls = Cell::new(0);
    let fetch = offline(&calls);

    // The daemon's start attempts and records the failure and its retry.
    let failed = store
        .ensure_with(Trigger::Start, &|| 1_000, &fetch)
        .unwrap_err();
    assert_eq!(failed.kind(), io::ErrorKind::NotConnected);
    assert_eq!(calls.get(), 1);
    let record = store.record().unwrap();
    assert_eq!(
        (record.failures, record.failed_at, record.retry_at),
        (1, 1_000, 1_030)
    );
    let report = store.report();
    assert_eq!(report.state, OpenH264State::Failed);
    assert_eq!(report.reason.as_deref(), Some("no answer"));
    assert_eq!(report.retry_at, Some(1_030));
    assert!(!report.library_present);

    // A session right after the failure does not wait for another attempt;
    // after the gap it tries at once, and the schedule doubles.
    assert!(store
        .ensure_with(Trigger::Session, &|| 1_010, &fetch)
        .is_err());
    assert_eq!(calls.get(), 1);
    assert!(store
        .ensure_with(Trigger::Session, &|| 1_040, &fetch)
        .is_err());
    assert_eq!(calls.get(), 2);
    let record = store.record().unwrap();
    assert_eq!((record.failures, record.retry_at), (2, 1_100));

    // A download that is not Cisco's file is refused; nothing is stored.
    let foreign = |_: &OpenH264Pin| -> io::Result<Vec<u8>> { Ok(b"not the cisco file".to_vec()) };
    let refused = store
        .ensure_with(Trigger::Start, &|| 1_200, &foreign)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    assert!(!store.library_present());
    assert_eq!(store.record().unwrap().failures, 3);

    // A session installs on demand only where no other H.264 coder serves.
    let installs = Cell::new(0);
    let install = |_: &OpenH264Store| -> io::Result<PathBuf> {
        installs.set(installs.get() + 1);
        Ok(PathBuf::new())
    };
    assert!(!store.session_ready(|| true, install));
    assert_eq!(installs.get(), 0);
    assert!(store.session_ready(|| false, install));
    assert_eq!(installs.get(), 1);
    assert!(!store.session_ready(|| false, |_| Err(io::Error::other("no answer"))));

    // While an installation holds the lock the state is „wird installiert“.
    let lock = InstallLock::acquire(&dir.path().join("openh264")).unwrap();
    assert_eq!(store.report().state, OpenH264State::Installing);
    drop(lock);

    // Switched off: nothing installs it and its coders offer nothing.
    store.disable().unwrap();
    assert!(store.opted_out());
    assert_eq!(store.report().state, OpenH264State::Disabled);
    assert!(!store.session_ready(|| false, |_| panic!("switched off")));
    let off = store
        .ensure_with(Trigger::Start, &|| 2_000, &fetch)
        .unwrap_err();
    assert_eq!(off.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(calls.get(), 2);
    let codecs = crate::screen::os::openh264_codec::codecs;
    let (encoders, decoders, quick) = codecs(store.clone(), false);
    assert_eq!(quick.state, OpenH264State::Disabled);
    assert_eq!((encoders.len(), decoders.len()), (1, 1));
    assert!(encoders[0].offers().is_empty() && decoders[0].offers().is_empty());
    let (encoders, decoders, _) = codecs(store.clone(), true);
    assert!(encoders.is_empty() && decoders.is_empty());

    // `remove` keeps it off; `enable` switches it on and tries at once.
    store.remove().unwrap();
    assert!(store.opted_out() && store.record().is_none());
    let wrong = store.enable_with(&"0".repeat(64), &fetch).unwrap_err();
    assert_eq!(wrong.kind(), io::ErrorKind::PermissionDenied);
    assert!(store.opted_out());
    assert!(store
        .enable_with(CISCO_BINARY_LICENSE_SHA256, &fetch)
        .is_err());
    assert_eq!(calls.get(), 3);
    assert!(!store.opted_out());
    assert_eq!(store.report().state, OpenH264State::Failed);
}
