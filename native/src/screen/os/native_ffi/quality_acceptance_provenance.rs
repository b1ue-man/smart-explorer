//! Test-only immutable release source and physical-library provenance.
use crate::screen::core::{
    quality_acceptance_baseline as baseline, quality_acceptance_metrics as metrics,
};
use serde_json::{json, Value};
use std::process::Command;

fn git(arguments: &[&str]) -> Vec<u8> {
    let result = Command::new("git").args(arguments).output().unwrap();
    assert!(
        result.status.success(),
        "quality provenance git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn original(path: &str) -> String {
    String::from_utf8(git(&[
        "show",
        &format!("{}:{path}", baseline::RELEASE_COMMIT),
    ]))
    .unwrap()
}
fn range<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let from = source.find(start).unwrap();
    &source[from..from + source[from..].find(end).unwrap()]
}
// Only embedded working-tree text uses this: immutable git blobs are hashed raw.
fn checkout_lf(source: &str) -> String {
    source.replace("\r\n", "\n")
}
fn frozen<'a>(source: &'a str, name: &str) -> &'a str {
    source
        .split_once(&format!("// BEGIN_FROZEN_{name}\n"))
        .unwrap()
        .1
        .split_once(&format!("// END_FROZEN_{name}"))
        .unwrap()
        .0
}
pub(super) fn verify() -> Value {
    let current = String::from_utf8(git(&["rev-parse", "HEAD"])).unwrap();
    let candidate = std::env::var("CANDIDATE_SHA").expect("common suite supplies CANDIDATE_SHA");
    assert_eq!(
        candidate,
        current.trim(),
        "quality evidence candidate and checkout must agree"
    );
    std::env::var_os("SMART_EXPLORER_TASK_LOG_ROOT")
        .expect("common suite supplies its artifact log root");
    let pixels = original(baseline::PIXEL_PATH);
    let codec = original(baseline::CODEC_PATH);
    assert_eq!(metrics::sha256(pixels.as_bytes()), baseline::PIXEL_SHA256);
    assert_eq!(metrics::sha256(codec.as_bytes()), baseline::CODEC_SHA256);
    let pixel_snapshot = checkout_lf(include_str!("../../core/quality_acceptance_baseline.rs"));
    assert_eq!(
        range(
            &pixels,
            "#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]\npub(crate) enum YuvMatrix",
            "/// NV12 → BGRA"
        ),
        frozen(&pixel_snapshot, "PIXELS")
    );
    let snapshot = checkout_lf(include_str!("quality_acceptance_baseline.rs"));
    assert_eq!(
        range(
            &codec,
            "/// One OpenH264 encoder;",
            "pub(crate) struct OpenH264DecoderFactory"
        ),
        frozen(&snapshot, "ENCODER")
    );
    assert_eq!(
        range(
            &codec,
            "/// One OpenH264 decoder of",
            "/// Serializes the task tests"
        ),
        frozen(&snapshot, "DECODER")
    );
    let lock = original("native/Cargo.lock");
    let current_lock = checkout_lf(include_str!("../../../../Cargo.lock"));
    for name in ["openh264", "openh264-sys2"] {
        let version = format!("name = \"{name}\"\nversion = \"0.9.9\"");
        assert!(lock.contains(&version));
        assert!(current_lock.contains(&version));
    }
    let pin = super::super::openh264_pins::PIN.unwrap();
    assert!(
        original("native/src/screen/os/native_ffi/openh264_pins.rs").contains(pin.library_sha256)
    );
    let rate = original("native/src/screen/core/rate_control.rs");
    assert!(rate.contains("pub(crate) const MIN_BITRATE_BPS: u64 = 150_000;"));
    assert!(rate.contains("pub(crate) const MAX_BITRATE_BPS: u64 = 240_000_000;"));
    json!({"release_commit": baseline::RELEASE_COMMIT,
        "pixel_source_sha256": baseline::PIXEL_SHA256, "codec_source_sha256": baseline::CODEC_SHA256,
        "source_blocks": "byte-identical to immutable git source",
        "openh264_crate": "0.9.9", "openh264_sys2_crate": "0.9.9",
        "library_sha256": pin.library_sha256,
        "baseline_scope": "frozen CPU codec path, one synchronous output from the first input, no pending decoder frames; not an old application build"})
}

#[test]
fn batches_task_b6_quality_provenance_checkout_line_endings_keep_exact_content() {
    for (source, name) in [
        (
            include_str!("../../core/quality_acceptance_baseline.rs"),
            "PIXELS",
        ),
        (include_str!("quality_acceptance_baseline.rs"), "ENCODER"),
        (include_str!("quality_acceptance_baseline.rs"), "DECODER"),
    ] {
        let lf = checkout_lf(source);
        let crlf = lf.replace('\n', "\r\n");
        let windows = checkout_lf(&crlf);
        assert_eq!(windows, lf);
        let expected = frozen(&lf, name);
        assert_eq!(frozen(&windows, name), expected);
        let marker = format!("// BEGIN_FROZEN_{name}\r\n");
        for inserted in ["// altered frozen source\r\n", " ", "\r"] {
            let changed = crlf.replacen(&marker, &format!("{marker}{inserted}"), 1);
            let changed_lf = checkout_lf(&changed);
            assert_ne!(
                frozen(&changed_lf, name),
                expected,
                "checkout conversion must retain inserted content in {name}"
            );
        }
    }
    let lock = checkout_lf(include_str!("../../../../Cargo.lock"));
    let windows_lock = checkout_lf(&lock.replace('\n', "\r\n"));
    assert_eq!(windows_lock, lock);
    for name in ["openh264", "openh264-sys2"] {
        let version = format!("name = \"{name}\"\nversion = \"0.9.9\"");
        assert!(windows_lock.contains(&version));
        let changed = format!("name = \"{name}\"\r\nversion = \"0.9.8\"");
        assert!(!checkout_lf(&changed).contains(&version));
    }
}
