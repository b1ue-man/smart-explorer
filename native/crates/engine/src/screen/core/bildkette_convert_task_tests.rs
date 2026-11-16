//! Task suite „Bildkette“ BK3b (plan 2026-11-14 B-19): the conversion on all
//! cores that OpenH264 and Media Foundation share is bit-identical to the
//! reference `bgra_to_i420_with_limits`, and the readback of a GPU picture
//! equals the reference NV12 → BGRA at the same size. Not registered yet
//! (`docs/tests/register.tsv`): the manager adds the rows.
use super::*;
use crate::screen::core::pixel_convert::{bgra_to_i420_with_limits, nv12_to_bgra, BgraImage};

/// A picture whose every pixel differs from its neighbours.
fn pattern(size: FrameSize, seed: u32) -> BgraImage {
    let mut data = vec![0u8; size.bgra_len()];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let value = (index as u32)
            .wrapping_mul(2_654_435_761)
            .wrapping_add(seed);
        pixel.copy_from_slice(&[value as u8, (value >> 8) as u8, (value >> 16) as u8, 255]);
    }
    BgraImage::from_vec(size, data).expect("pattern")
}

const SIZES: [(u32, u32); 8] = [
    (1, 1),
    (1, 3),
    (2, 2),
    (17, 9),
    (33, 31),
    (64, 36),
    (641, 361),
    (1920, 1080),
];

#[test]
fn bildkette_task_chain_openh264_conversion_is_bit_identical_to_the_reference() {
    for (index, (width, height)) in SIZES.into_iter().enumerate() {
        let size = FrameSize::new(width, height);
        let image = pattern(size, index as u32 + 1);
        for matrix in [YuvMatrix::Bt709, YuvMatrix::Bt601] {
            let reference = bgra_to_i420_with_limits(&image.view(), matrix, FrameLimits::LEGACY)
                .expect("reference");
            let parallel = bgra_to_i420_parallel(&image.view(), matrix, FrameLimits::LEGACY)
                .expect("parallel");
            assert_eq!(parallel, reference, "I420 {size:?} {matrix:?}");
            // NV12 of the same function: the same samples, interleaved.
            let chroma = size.chroma();
            let pitch = (width as usize).max(chroma.width as usize * 2) + 7;
            let mut y = vec![0u8; pitch * height as usize];
            let mut uv = vec![0u8; pitch * chroma.height as usize];
            bgra_to_nv12_parallel(&image.view(), matrix, &mut y, &mut uv, pitch).expect("NV12");
            for row in 0..height as usize {
                let tight = row * width as usize;
                assert_eq!(
                    &y[row * pitch..row * pitch + width as usize],
                    &reference.y[tight..tight + width as usize]
                );
            }
            for row in 0..chroma.height as usize {
                for column in 0..chroma.width as usize {
                    let at = row * chroma.width as usize + column;
                    let pair = &uv[row * pitch + column * 2..row * pitch + column * 2 + 2];
                    assert_eq!(pair, [reference.u[at], reference.v[at]], "NV12 {size:?}");
                }
            }
        }
    }
    // Planes too small for the picture are refused, not overrun.
    let image = pattern(FrameSize::new(8, 8), 9);
    let (mut y, mut uv) = (vec![0u8; 63], vec![0u8; 32]);
    assert!(bgra_to_nv12_parallel(&image.view(), YuvMatrix::Bt709, &mut y, &mut uv, 8).is_err());
}

#[test]
fn bildkette_task_chain_gpu_readback_matches_the_reference_conversion() {
    let size = FrameSize::new(64, 36);
    let image = pattern(size, 3);
    let chroma = size.chroma();
    let mut y = vec![0u8; size.pixels() as usize];
    let mut uv = vec![0u8; chroma.pixels() as usize * 2];
    bgra_to_nv12_parallel(&image.view(), YuvMatrix::Bt709, &mut y, &mut uv, 64).expect("NV12");
    let view = Nv12View::new(size, &y, 64, &uv, 64).expect("view");
    let mut reference = BgraImage::new(size).expect("target");
    nv12_to_bgra(&view, YuvMatrix::Bt709, &mut reference).expect("reference");
    let mut read = vec![0u8; size.bgra_len()];
    nv12_to_bgra_resampled(&view, YuvMatrix::Bt709, &mut read, size).expect("readback");
    assert_eq!(
        read,
        reference.data(),
        "same size: the reference conversion"
    );
    // One column and row more than the NV12 surface (odd capture): the edge
    // repeats; a smaller target takes the nearest samples.
    let odd = FrameSize::new(65, 37);
    let mut wider = vec![0u8; odd.bgra_len()];
    nv12_to_bgra_resampled(&view, YuvMatrix::Bt709, &mut wider, odd).expect("odd");
    assert_eq!(&wider[64 * 4..65 * 4], &reference.data()[63 * 4..64 * 4]);
    let half = FrameSize::new(32, 18);
    let mut small = vec![0u8; half.bgra_len()];
    nv12_to_bgra_resampled(&view, YuvMatrix::Bt709, &mut small, half).expect("half");
    assert_eq!(&small[..4], &reference.data()[..4]);
    assert!(nv12_to_bgra_resampled(&view, YuvMatrix::Bt709, &mut small, size).is_err());
}
