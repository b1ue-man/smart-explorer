//! Test-only V7 full-reference measurements; no codec or transport is simulated here.
use super::{decoded_frame::DecodedFrame, pixel_convert::BgraImage, wire::invalid};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io;

pub(crate) const METHOD: &str = "RGB PSNR, peak 255; BT.709 full-range luma SSIM, valid 11x11 Gaussian sigma=1.5, K1=.01 K2=.03, stride 1, no downsampling; alpha counted separately";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Quality {
    pub pixels: u64,
    pub different_pixels: u64,
    pub different_alpha_pixels: u64,
    pub rgb_squared_error: u64,
    /// None means +infinity ONLY when psnr_infinite is true; never missing output.
    pub psnr_rgb_db: Option<f64>,
    pub psnr_infinite: bool,
    pub ssim_luma: f64,
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn luma(pixel: &[u8]) -> f64 {
    0.0722 * f64::from(pixel[0]) + 0.7152 * f64::from(pixel[1]) + 0.2126 * f64::from(pixel[2])
}

/// Separable Gaussian moments produce the same valid-window statistics as the
/// normalized 11x11 kernel of Wang et al. (2004), equations 12–17.
fn ssim(reference: &BgraImage, output: &BgraImage) -> f64 {
    let size = reference.size();
    let (width, height) = (size.width as usize, size.height as usize);
    let columns = width - 10;
    let mut kernel = [0.0; 11];
    for (index, weight) in kernel.iter_mut().enumerate() {
        *weight = (-((index as f64 - 5.0).powi(2)) / (2.0 * 1.5_f64.powi(2))).exp();
    }
    let normalizer: f64 = kernel.iter().sum();
    for weight in &mut kernel {
        *weight /= normalizer;
    }
    // Eleven horizontal rows suffice; retained metric scratch is O(width).
    let mut rows = vec![vec![[0.0; 5]; columns]; 11];
    let (c1, c2) = ((0.01_f64 * 255.0).powi(2), (0.03_f64 * 255.0).powi(2));
    let mut total = 0.0;
    for y in 0..height {
        let a = reference.view().row(y as u32);
        let b = output.view().row(y as u32);
        for x in 0..columns {
            let mut moments = [0.0; 5];
            for (offset, weight) in kernel.iter().enumerate() {
                let at = (x + offset) * 4;
                let (a, b) = (luma(&a[at..at + 4]), luma(&b[at..at + 4]));
                for (sum, value) in moments.iter_mut().zip([a, b, a * a, b * b, a * b]) {
                    *sum += weight * value;
                }
            }
            rows[y % 11][x] = moments;
        }
        if y < 10 {
            continue;
        }
        for x in 0..columns {
            let mut moments = [0.0; 5];
            for (offset, weight) in kernel.iter().enumerate() {
                for (sum, value) in moments.iter_mut().zip(rows[(y + offset + 1) % 11][x]) {
                    *sum += weight * value;
                }
            }
            let [a, b, a2, b2, ab] = moments;
            let va = (a2 - a * a).max(0.0);
            let vb = (b2 - b * b).max(0.0);
            total += ((2.0 * a * b + c1) * (2.0 * (ab - a * b) + c2))
                / ((a * a + b * b + c1) * (va + vb + c2));
        }
    }
    total / (columns * (height - 10)) as f64
}

pub(crate) fn measure(reference: &BgraImage, output: &BgraImage) -> io::Result<Quality> {
    let size = reference.size();
    if output.size() != size || size.width < 11 || size.height < 11 {
        return Err(invalid(
            "quality comparison requires matching capture geometry of at least 11x11",
        ));
    }
    let mut result = Quality {
        pixels: size.pixels(),
        different_pixels: 0,
        different_alpha_pixels: 0,
        rgb_squared_error: 0,
        psnr_rgb_db: None,
        psnr_infinite: false,
        ssim_luma: 0.0,
    };
    for (a, b) in reference
        .data()
        .chunks_exact(4)
        .zip(output.data().chunks_exact(4))
    {
        result.different_pixels += u64::from(a != b);
        result.different_alpha_pixels += u64::from(a[3] != b[3]);
        for channel in 0..3 {
            let error = i64::from(a[channel]) - i64::from(b[channel]);
            result.rgb_squared_error += (error * error) as u64;
        }
    }
    result.psnr_infinite = result.rgb_squared_error == 0;
    if !result.psnr_infinite {
        let mse = result.rgb_squared_error as f64 / (result.pixels * 3) as f64;
        result.psnr_rgb_db = Some(10.0 * (255.0_f64.powi(2) / mse).log10());
    }
    result.ssim_luma = if result.psnr_infinite {
        1.0
    } else {
        ssim(reference, output)
    };
    if !result.ssim_luma.is_finite() {
        return Err(invalid("nonfinite SSIM measurement"));
    }
    Ok(result)
}

/// Frame/capture time/key status and geometry are checked before comparing.
pub(crate) fn paired(
    reference: &BgraImage,
    identity: DecodedFrame,
    baseline: (&BgraImage, DecodedFrame),
    candidate: (&BgraImage, DecodedFrame),
) -> io::Result<(Quality, Quality)> {
    if baseline.1 != identity || candidate.1 != identity {
        return Err(invalid(
            "quality outputs do not belong to the same capture frame",
        ));
    }
    Ok((
        measure(reference, baseline.0)?,
        measure(reference, candidate.0)?,
    ))
}

/// Only for an unchanged codec configuration and identical capture cadence.
/// The tiny epsilon absorbs floating summation noise, not a quality budget.
pub(crate) fn no_codec_regression(before: &Quality, after: &Quality) -> bool {
    after.rgb_squared_error <= before.rgb_squared_error
        && after.ssim_luma + 1e-12 >= before.ssim_luma
        && after.different_alpha_pixels <= before.different_alpha_pixels
}

pub(crate) fn retain(label: &str, image: &BgraImage) -> serde_json::Value {
    assert!(label
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    let digest = sha256(image.data());
    let relative = format!("quality-acceptance/frames/{label}.bgra");
    if let Some(root) = std::env::var_os("SMART_EXPLORER_TASK_LOG_ROOT") {
        let path = std::path::PathBuf::from(root).join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, image.data()).unwrap();
    }
    serde_json::json!({"sha256": digest, "width": image.size().width, "height": image.size().height,
        "format": "tight BGRA8, opaque capture", "artifact": relative})
}

/// Optional artifacts use the existing single-suite log root. Missing candidate
/// identity is recorded as unknown for standalone mathematical fixtures.
pub(crate) fn evidence(label: &str, body: serde_json::Value) {
    assert!(label
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    let report = serde_json::json!({"schema": 1, "fixture": label,
        "candidate": std::env::var("CANDIDATE_SHA").ok(), "method": METHOD, "observations": body});
    println!("SE_TASK_QUALITY_JSON={report}");
    if let Some(root) = std::env::var_os("SMART_EXPLORER_TASK_LOG_ROOT") {
        let directory = std::path::PathBuf::from(root).join("quality-acceptance");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{label}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
