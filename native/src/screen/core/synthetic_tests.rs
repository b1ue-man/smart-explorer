use super::*;
use crate::screen::core::pixel_convert::downscale_bgra;

fn rendered(frame_id: u64, size: FrameSize) -> BgraImage {
    let mut image = BgraImage::new(size).unwrap();
    render(frame_id, &mut image).unwrap();
    image
}

#[test]
fn screen_task_barcode_detects_corruption() {
    // CRC-32 check value of the IEEE polynomial.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);

    // Every size from the minimum to portrait reads back its frame id.
    for size in [
        FrameSize::new(128, 72),
        FrameSize::new(640, 360),
        FrameSize::new(1080, 1920),
    ] {
        for frame_id in [0, 1, 0x0123_4567_89AB_CDEF, u64::MAX] {
            let image = rendered(frame_id, size);
            assert_eq!(
                read_barcode(&image.view(), size).unwrap(),
                BarcodeRead::Valid(frame_id)
            );
        }
    }

    // Each single wrong cell is a corrupt frame (CRC-32 finds every
    // single-bit error).
    let size = FrameSize::new(640, 360);
    let frame_id = 0x0123_4567_89AB_CDEF;
    let image = rendered(frame_id, size);
    let layout = Layout::of(size);
    let bits = barcode_bits(frame_id);
    for index in 0..BARCODE_COLUMNS * BARCODE_ROWS {
        let mut corrupt = image.clone();
        let x = layout.margin + (index % BARCODE_COLUMNS) * layout.cell;
        let y = layout.margin + (index / BARCODE_COLUMNS) * layout.cell;
        let inverted = if (bits >> (95 - index)) & 1 == 1 {
            BLACK
        } else {
            WHITE
        };
        fill(
            &mut corrupt,
            i64::from(x),
            i64::from(y),
            layout.cell,
            layout.cell,
            inverted,
        );
        assert_eq!(
            read_barcode(&corrupt.view(), size).unwrap(),
            BarcodeRead::Corrupt,
            "cell {index}"
        );
    }

    // No false alarm: the barcode survives the resolution ladder (× 0.75)
    // and noise of ±30 per channel, as lossy coding leaves it.
    let scaled = downscale_bgra(&image.view(), size.scale(3, 4)).unwrap();
    assert_eq!(
        read_barcode(&scaled.view(), size).unwrap(),
        BarcodeRead::Valid(frame_id)
    );
    let mut noisy = image.clone();
    let mut state = 0x1234_5678u32;
    for byte in noisy.data_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let delta = (state % 61) as i16 - 30;
        *byte = (i16::from(*byte) + delta).clamp(0, 255) as u8;
    }
    assert_eq!(
        read_barcode(&noisy.view(), size).unwrap(),
        BarcodeRead::Valid(frame_id)
    );

    // Frames below 128 × 72 have no room for the barcode.
    let mut tiny = BgraImage::new(FrameSize::new(127, 72)).unwrap();
    assert!(render(1, &mut tiny).is_err());
    assert!(read_barcode(&tiny.view(), FrameSize::new(127, 72)).is_err());
}
