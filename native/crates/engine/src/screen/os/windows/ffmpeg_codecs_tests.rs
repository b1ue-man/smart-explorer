//! P8 (plan 2026-11-17): FFmpeg's place in the Windows decoder register and
//! Media Foundation where the FFmpeg DLLs are absent.
use std::io;
use std::sync::Arc;

use super::*;
use crate::screen::core::ffmpeg_plan::AVCODEC_MAJOR;

/// A Media Foundation decoder as `mf_common::codecs` lists one.
struct FakeTransform;

impl DecoderFactory for FakeTransform {
    fn name(&self) -> String {
        "Microsoft H264 Video Decoder MFT".into()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::h264(H264Profile::High, false)]
    }

    fn open(&self, _header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        Err(io::Error::other("no transform in this test"))
    }
}

/// FFmpeg's three factories stand before the Media Foundation decoders;
/// under „Automatisch“ (the process default) Media Foundation keeps its
/// offers, and without the DLLs FFmpeg offers nothing, so the viewer's hello
/// is the one of Media Foundation alone (the GitHub runners have no DLLs).
#[test]
fn bildkette_task_ffmpeg_absent_dlls_leave_media_foundation() {
    let decoders = decoders(vec![Arc::new(FakeTransform)]);
    let names: Vec<String> = decoders.iter().map(|factory| factory.name()).collect();
    assert_eq!(names.len(), 4);
    assert_eq!(
        names[..3],
        [
            "FFmpeg h264 (D3D11VA/DXVA2/Software)",
            "FFmpeg hevc (D3D11VA/DXVA2/Software)",
            "FFmpeg av1 (D3D11VA/DXVA2/Software)",
        ]
    );
    assert_eq!(names[3], "Microsoft H264 Video Decoder MFT");
    assert_eq!(
        decoders[3].offers(),
        vec![CodecOffer::h264(H264Profile::High, false)]
    );
    assert!(!decoders[0].keeps_references(CodecKind::H264));
    assert!(decoders[1].keeps_references(CodecKind::Hevc));
    if !ffmpeg_ffi::present() {
        assert!(decoders[..3]
            .iter()
            .all(|factory| factory.offers().is_empty()));
    }
}

/// On a device with the installed or downloaded DLLs: the pinned build
/// loads, its layout fingerprints and pixel formats check out, the viewer
/// device exists and FFmpeg offers at least its software H.264 decoder.
#[test]
#[ignore = "FFmpeg-DLLs (Installation oder App-Daten) und eine GPU; Suite keine"]
fn bildkette_task_ffmpeg_device_loads_the_pinned_build() {
    let ffmpeg = ffmpeg_ffi::ffmpeg().expect("the pinned FFmpeg build loads");
    assert!(ffmpeg.formats.valid());
    assert!(plan::version_matches(ffmpeg.versions.1, AVCODEC_MAJOR));
    let device = super::super::viewer_d3d11::shared().expect("viewer device");
    assert!(!device.lost());
    let decoders = decoders(Vec::new());
    assert!(!decoders[0].offers().is_empty());
}
