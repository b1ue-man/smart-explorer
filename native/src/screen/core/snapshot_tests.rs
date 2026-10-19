//! Acceptance of the still picture (SCR-M4): the PNG is lossless, the fit
//! respects a byte budget, and the tile records of a snapshot stream build
//! the same picture again.
use std::io::Cursor;

use super::*;
use crate::screen::core::tiles_codec::{TileEncoder, MAX_TILES_RECORD_BYTES};
use crate::screen::core::wire_binary::{FLAG_KEYFRAME, FLAG_MORE};

/// A picture with structure at every scale: a gradient, a checker board and
/// a pseudo-random band that does not compress.
fn picture(size: FrameSize) -> BgraImage {
    let mut image = BgraImage::new(size).unwrap();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for y in 0..size.height {
        for (x, pixel) in image.row_mut(y).chunks_exact_mut(4).enumerate() {
            let x = x as u32;
            let noise = y > size.height / 2;
            let value = if noise {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            } else {
                (x ^ y) as u8
            };
            pixel.copy_from_slice(&[
                value,
                (x * 255 / size.width.max(1)) as u8,
                (y * 255 / size.height.max(1)) as u8,
                255,
            ]);
        }
    }
    image
}

fn decode_png(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    (info, buffer)
}

#[test]
fn screen_task_snapshot_png_roundtrip() {
    let size = FrameSize::new(203, 117);
    let image = picture(size);
    let png = encode_png(&image).unwrap();
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
    let (info, pixels) = decode_png(&png);
    assert_eq!((info.width, info.height), (203, 117));
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    for (rgb, bgra) in pixels.chunks_exact(3).zip(image.data().chunks_exact(4)) {
        assert_eq!(rgb, [bgra[2], bgra[1], bgra[0]]);
    }
    // The tile records of a snapshot stream give the same picture.
    let mut encoder = TileEncoder::new(size).unwrap();
    let payloads = encoder
        .encode(&image.view(), true, MAX_TILES_RECORD_BYTES)
        .unwrap();
    let count = payloads.len();
    let records: Vec<FrameRecord> = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| FrameRecord {
            frame_id: 7,
            capture_ts_us: 0,
            flags: if index + 1 < count {
                FLAG_KEYFRAME | FLAG_MORE
            } else {
                FLAG_KEYFRAME
            },
            payload,
        })
        .collect();
    assert_eq!(decode_snapshot(&records, size).unwrap(), image);
    assert!(decode_snapshot(&[], size).is_err());
    let mut open = records;
    if let Some(last) = open.last_mut() {
        last.flags |= FLAG_MORE;
    }
    assert!(decode_snapshot(&open, size).is_err());
}

#[test]
fn screen_task_snapshot_fit_respects_max_bytes() {
    let image = picture(FrameSize::new(640, 360));
    let whole = fit(&image, None, None).unwrap();
    assert_eq!(whole.size, FrameSize::new(640, 360));
    // A long edge is honoured with the aspect ratio kept.
    let edge = fit(&image, Some(320), None).unwrap();
    assert_eq!(edge.size, FrameSize::new(320, 180));
    // A budget below the whole picture shrinks it in steps of 3/4, from
    // the original, until the PNG fits.
    let budget = whole.png.len() / 3;
    let small = fit(&image, None, Some(budget)).unwrap();
    assert!(small.png.len() <= budget, "{} > {budget}", small.png.len());
    assert!(small.size.width < 640 && small.size.height < 360);
    let (info, _) = decode_png(&small.png);
    assert_eq!(
        (info.width, info.height),
        (small.size.width, small.size.height)
    );
    // A budget no picture of 64 pixels or more can meet is refused.
    let refused = fit(&image, None, Some(16)).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
    assert!(refused.to_string().contains("max_bytes"));
    assert_eq!(
        fit(&image, Some(0), None).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    // A picture that already fits stays as it is.
    let kept = fit(&image, Some(4096), Some(whole.png.len())).unwrap();
    assert_eq!(kept.size, whole.size);
}
