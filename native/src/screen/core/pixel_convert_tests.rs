use super::*;

/// BGR colours, among them the extremes and saturated primaries.
const COLOURS: [[u8; 3]; 10] = [
    [0, 0, 0],
    [255, 255, 255],
    [0, 0, 255],
    [0, 255, 0],
    [255, 0, 0],
    [128, 128, 128],
    [30, 200, 90],
    [250, 10, 128],
    [1, 2, 3],
    [254, 253, 252],
];

/// An odd-sized picture of uniform 2 × 2 blocks, so 4:2:0 loses nothing.
fn blocks(size: FrameSize) -> Vec<u8> {
    let mut data = vec![0u8; size.bgra_len()];
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let (x, y) = (index as u32 % size.width, index as u32 / size.width);
        let colour = COLOURS[((x / 2 + 3 * (y / 2)) as usize) % COLOURS.len()];
        pixel[..3].copy_from_slice(&colour);
        pixel[3] = 0x7F;
    }
    data
}

#[test]
fn screen_task_pixel_convert_yuv_round_trip() {
    let size = FrameSize::new(15, 7);
    let source = blocks(size);
    let view = BgraView::new(size, 60, &source).unwrap();
    for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
        let yuv = bgra_to_i420(&view, matrix).unwrap();
        assert_eq!((yuv.y.len(), yuv.u.len(), yuv.v.len()), (105, 32, 32));
        let mut back = BgraImage::new(size).unwrap();
        i420_to_bgra(&yuv, matrix, &mut back).unwrap();
        for (before, after) in source.chunks_exact(4).zip(back.data().chunks_exact(4)) {
            for (old, new) in before[..3].iter().zip(&after[..3]) {
                assert!(
                    old.abs_diff(*new) <= 2,
                    "{matrix:?}: {before:?} became {after:?}"
                );
            }
            assert_eq!(after[3], 0xFF);
        }
        // NV12 with padded rows gives the same picture as I420.
        let chroma = size.chroma();
        let (y_stride, uv_stride) = (16, 20);
        let mut y_plane = vec![0u8; y_stride * 7];
        for (row, line) in yuv.y.chunks_exact(15).enumerate() {
            y_plane[row * y_stride..row * y_stride + 15].copy_from_slice(line);
        }
        let mut uv_plane = vec![0u8; uv_stride * chroma.height as usize];
        for (index, (u, v)) in yuv.u.iter().zip(&yuv.v).enumerate() {
            let (cx, cy) = (index % 8, index / 8);
            uv_plane[cy * uv_stride + cx * 2] = *u;
            uv_plane[cy * uv_stride + cx * 2 + 1] = *v;
        }
        let nv12 = Nv12View::new(size, &y_plane, y_stride, &uv_plane, uv_stride).unwrap();
        let mut from_nv12 = BgraImage::new(size).unwrap();
        nv12_to_bgra(&nv12, matrix, &mut from_nv12).unwrap();
        assert_eq!(from_nv12, back);
        assert!(Nv12View::new(size, &y_plane, 14, &uv_plane, uv_stride).is_err());
    }
    // Limited range: black and white map to 16 and 235.
    let corners = [0, 0, 0, 0, 255, 255, 255, 0];
    let pair = BgraView::new(FrameSize::new(2, 1), 8, &corners).unwrap();
    let yuv = bgra_to_i420(&pair, YuvMatrix::Bt709).unwrap();
    assert_eq!(yuv.y, [16, 235]);
    assert_eq!((yuv.u[0], yuv.v[0]), (128, 128));
    let mut wrong = BgraImage::new(FrameSize::new(3, 1)).unwrap();
    assert!(i420_to_bgra(&yuv, YuvMatrix::Bt709, &mut wrong).is_err());
}

#[test]
fn screen_task_pixel_convert_downscale_and_sizes() {
    // 4 × 2 → 2 × 1 averages each 2 × 2 block.
    let row = [
        10, 20, 30, 255, 30, 40, 50, 255, 100, 100, 100, 255, 201, 201, 201, 255,
    ];
    let data = [row, row].concat();
    let view = BgraView::new(FrameSize::new(4, 2), 16, &data).unwrap();
    let small = downscale_bgra(&view, FrameSize::new(2, 1)).unwrap();
    assert_eq!(small.data(), [20, 30, 40, 255, 151, 151, 151, 255]);
    assert_eq!(
        downscale_bgra(&view, FrameSize::new(4, 2)).unwrap().data(),
        data.as_slice()
    );
    assert!(downscale_bgra(&view, FrameSize::new(5, 2)).is_err());
    // Buffers that do not match their size and pitch are refused.
    assert!(BgraView::new(FrameSize::new(4, 2), 15, &data).is_err());
    assert!(BgraView::new(FrameSize::new(4, 3), 16, &data).is_err());
    assert!(BgraView::new(FrameSize::new(0, 2), 16, &data).is_err());
    assert!(BgraImage::from_vec(FrameSize::new(4, 2), data[..31].to_vec()).is_err());
    // Fitting keeps the aspect ratio and never enlarges.
    let max = FrameSize::MAX;
    assert_eq!(
        FrameSize::new(7680, 4320).fit_within(max),
        FrameSize::new(4096, 2304)
    );
    assert_eq!(
        FrameSize::new(1080, 2400).fit_within(max),
        FrameSize::new(1036, 2304)
    );
    assert_eq!(
        FrameSize::new(1280, 720).fit_within(max),
        FrameSize::new(1280, 720)
    );
    assert_eq!(
        FrameSize::new(1921, 1081).even(),
        FrameSize::new(1920, 1080)
    );
    assert_eq!(FrameSize::new(1, 1).even(), FrameSize::new(2, 2));
    assert_eq!(
        FrameSize::new(1920, 1080).scale(3, 4),
        FrameSize::new(1440, 810)
    );
    assert_eq!(FrameSize::new(15, 7).chroma(), FrameSize::new(8, 4));
    // Allocation limits for data from the network.
    assert!(FrameSize::new(4096, 2304).is_valid_frame());
    assert!(FrameSize::new(2304, 4096).is_valid_frame());
    assert!(!FrameSize::new(4096, 2305).is_valid_frame());
    assert!(!FrameSize::new(4097, 1).is_valid_frame());
    assert!(!FrameSize::new(0, 1).is_valid_frame());
    assert!(BgraImage::new(FrameSize::new(4096, 4096)).is_err());
}
