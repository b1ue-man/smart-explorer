//! BK4 task tests of the software AV1 hulls (`bildkette_task_linux_hw_av1_*`,
//! B-25/B-38): the loaders report "absent", the SVT-AV1 settings map B-21,
//! and with both libraries installed an encoded test picture decodes above
//! a PSNR threshold (ignored without them; the suite installs them).
use super::{
    svt_abi, svt_library, svt_settings, svt_version_supported, Dav1dDecoder, SvtAbi, SvtAv1Encoder,
    DAV1D_LIBRARIES,
};
use crate::screen::core::pixel_convert::{FrameSize, I420Image};

#[test]
fn bildkette_task_linux_hw_av1_loaders_report_absent() {
    let size = FrameSize::new(320, 180);
    let error = SvtAv1Encoder::open("libSvtAv1Enc-bk4-absent.so.1", size, 60, 2_000_000)
        .err()
        .expect("a missing SVT-AV1 must not load");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    let error = Dav1dDecoder::open(&["libdav1d-bk4-absent.so.7"])
        .err()
        .expect("a missing dav1d must not load");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn bildkette_task_linux_hw_av1_settings() {
    let settings = svt_settings(FrameSize::new(1920, 1080), 120, 30_000_000);
    let value = |name: &str| {
        settings
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(value("rc").as_deref(), Some("cbr"), "CBR selects low delay");
    assert_eq!(value("tbr").as_deref(), Some("30000000b"));
    assert_eq!(value("keyint").as_deref(), Some("-1"), "no periodic key");
    assert_eq!(value("lookahead").as_deref(), Some("0"));
    assert!(
        value("enable-force-key-frames").is_none(),
        "SVT-AV1 1.7 CBR low-delay rejects enable-force-key-frames"
    );
    assert_eq!(value("fps-num").as_deref(), Some("120"));
    assert_eq!(value("matrix-coefficients").as_deref(), Some("1"), "BT.709");
    let capped = svt_settings(FrameSize::new(64, 64), 30, 900_000_000);
    assert!(capped.contains(&("tbr", "100000000b".to_string())));
    assert_eq!(
        value("pred-struct").as_deref(),
        Some("1"),
        "low delay: packet per picture"
    );
    // 2.3.0 is the first with one packet per picture in low delay; 3.0
    // changed `svt_av1_enc_init_handle` and the buffer header.
    assert_eq!(svt_abi("v2.3.0"), Some(SvtAbi::V2));
    assert_eq!(svt_abi("2.3.1-12-gabc"), Some(SvtAbi::V2));
    assert_eq!(svt_abi("v3.1.0"), Some(SvtAbi::V3));
    assert_eq!(svt_abi("v4.3.0"), Some(SvtAbi::V3));
    for late in [
        "v1.7.0", "1.5.0", "v1.4.1", "v2.0.0", "v2.2.1", "v5.0.0", "",
    ] {
        assert!(!svt_version_supported(late), "{late}");
    }
}

/// A moving gradient with sharp edges, as a desktop shows it.
fn picture(size: FrameSize, frame: u32) -> I420Image {
    let chroma = size.chroma();
    let (w, h) = (size.width as usize, size.height as usize);
    let mut y = vec![0u8; w * h];
    for row in 0..h {
        for column in 0..w {
            let edge = if (column / 16 + row / 16).is_multiple_of(2) {
                40
            } else {
                0
            };
            y[row * w + column] = (16 + (column + row + frame as usize * 4) % 180 + edge) as u8;
        }
    }
    let plane = vec![128u8; (chroma.width * chroma.height) as usize];
    I420Image {
        size,
        y,
        u: plane.clone(),
        v: plane,
    }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let error: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    if error == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0 * 255.0 / error).log10()
}

#[test]
#[ignore = "SVT-AV1 2.3.0 or later (libSvtAv1Enc.so.2/3/4) and libdav1d.so.7; Suite crates-task"]
fn bildkette_task_linux_hw_av1_roundtrip_psnr() {
    let size = FrameSize::new(320, 180);
    let library = svt_library().expect("SVT-AV1 2.3.0 or later (the suite builds v3.1.0)");
    let mut encoder = SvtAv1Encoder::open(library, size, 60, 2_000_000).unwrap();
    let mut decoder = Dav1dDecoder::open(&DAV1D_LIBRARIES).unwrap();
    let mut shown = 0;
    for frame in 0..5 {
        let source = picture(size, frame);
        let unit = encoder.encode(&source, frame == 0).unwrap();
        assert!(!unit.is_empty(), "frame {frame} gave no temporal unit");
        if let Some(decoded) = decoder.decode(&unit).unwrap() {
            assert_eq!(decoded.size, size);
            let quality = psnr(&source.y, &decoded.y);
            assert!(quality > 30.0, "frame {frame}: PSNR {quality:.1} dB");
            shown += 1;
        }
    }
    assert!(shown >= 4, "only {shown} of 5 pictures decoded at once");
}
