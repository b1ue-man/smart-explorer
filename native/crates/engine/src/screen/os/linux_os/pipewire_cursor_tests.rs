//! The pointer metadata of a Wayland screen cast without PipeWire (plan
//! 2026-11-14, package BK8a): the parser of the `SPA_META_Cursor` block, the
//! decision "overlay or mix in" and the two ways to hand the pointer on.
use super::meta::{
    blend_premultiplied, parse_cursor_meta, BitmapUpdate, CursorMeta, PointerBitmap,
};
use super::{route, Pointer, PointerFeed, PointerRoute};
use crate::screen::core::cursor_overlay::{CursorUpdate, WorkerCursor};
use crate::screen::core::pixel_convert::FrameSize;

/// A `spa_meta_cursor` block: header, then (when `bitmap` is given) a
/// `spa_meta_bitmap` at offset 28 whose pixels follow at offset 20 of it.
/// `bitmap` = (format, width, height, stride, pixel bytes).
fn block(
    id: u32,
    position: (i32, i32),
    hot: (i32, i32),
    bitmap: Option<(u32, u32, u32, i32, &[u8])>,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut word = |value: u32| out.extend_from_slice(&value.to_le_bytes());
    word(id);
    word(0); // flags
    word(position.0 as u32);
    word(position.1 as u32);
    word(hot.0 as u32);
    word(hot.1 as u32);
    word(if bitmap.is_some() { 28 } else { 0 });
    if let Some((format, width, height, stride, pixels)) = bitmap {
        for value in [format, width, height, stride as u32, 20] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(pixels);
    }
    out
}

fn new_bitmap(width: u32, height: u32, hot: (i32, i32), rgba: Vec<u8>) -> CursorMeta {
    CursorMeta {
        id: 1,
        x: 0,
        y: 0,
        hot_x: hot.0,
        hot_y: hot.1,
        bitmap: BitmapUpdate::New {
            width,
            height,
            rgba,
        },
    }
}

fn moved(id: u32, x: i32, y: i32) -> CursorMeta {
    CursorMeta {
        id,
        x,
        y,
        hot_x: 0,
        hot_y: 0,
        bitmap: BitmapUpdate::Unchanged,
    }
}

#[test]
fn bildkette_task_wayland_cursor_meta_parser_reads_position_hot_spot_and_bitmap() {
    // RGBA, 2 x 1, stride 12 (4 bytes of padding): opaque red, half-transparent green.
    let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 9, 9, 9, 9];
    let bytes = block(7, (100, -5), (1, 2), Some((11, 2, 1, 12, &rgba)));
    let meta = parse_cursor_meta(&bytes).unwrap();
    assert_eq!((meta.id, meta.x, meta.y), (7, 100, -5));
    assert_eq!((meta.hot_x, meta.hot_y), (1, 2));
    // Colors come out premultiplied (255 * 128 / 255 = 128), the padding is not read.
    assert_eq!(
        meta.bitmap,
        BitmapUpdate::New {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 128, 0, 128]
        }
    );
    // The same pixels in the other byte orders (`spa_video_format` 12, 13, 14, 8).
    let red = BitmapUpdate::New {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
    };
    for (format, pixel) in [
        (12u32, [0u8, 0, 255, 255]), // BGRA
        (13, [255, 255, 0, 0]),      // ARGB
        (14, [255, 0, 0, 255]),      // ABGR
        (8, [0, 0, 255, 0]),         // BGRx: the fourth byte is unused, alpha is 255
    ] {
        let bytes = block(1, (0, 0), (0, 0), Some((format, 1, 1, 4, &pixel)));
        assert_eq!(
            parse_cursor_meta(&bytes).unwrap().bitmap,
            red,
            "format {format}"
        );
    }
}

#[test]
fn bildkette_task_wayland_cursor_meta_parser_tells_hidden_unchanged_and_empty_apart() {
    // id 0: no pointer; no bitmap block: the shape held stays.
    let hidden = parse_cursor_meta(&block(0, (3, 4), (0, 0), None)).unwrap();
    assert_eq!(hidden.id, 0);
    assert_eq!(hidden.bitmap, BitmapUpdate::Unchanged);
    // Bitmap format 0: "no new bitmap".
    let none = parse_cursor_meta(&block(1, (0, 0), (0, 0), Some((0, 0, 0, 0, &[])))).unwrap();
    assert_eq!(none.bitmap, BitmapUpdate::Unchanged);
    // Bitmap offset 0: the pointer has no image (invisible).
    let mut invisible = block(1, (0, 0), (0, 0), Some((11, 1, 1, 4, &[1, 2, 3, 4])));
    invisible[28 + 16..28 + 20].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        parse_cursor_meta(&invisible).unwrap().bitmap,
        BitmapUpdate::Empty
    );
    // An unknown format and an image above the overlay's limit are no pointer.
    let unknown = block(1, (0, 0), (0, 0), Some((77, 1, 1, 4, &[0; 4])));
    assert_eq!(
        parse_cursor_meta(&unknown).unwrap().bitmap,
        BitmapUpdate::Empty
    );
    let wide = vec![0u8; 513 * 4];
    let large = block(1, (0, 0), (0, 0), Some((11, 513, 1, 513 * 4, &wide)));
    assert_eq!(
        parse_cursor_meta(&large).unwrap().bitmap,
        BitmapUpdate::Empty
    );
}

#[test]
fn bildkette_task_wayland_cursor_meta_parser_rejects_ranges_outside_the_block() {
    let pixels = [255u8; 8];
    let good = block(1, (0, 0), (0, 0), Some((11, 2, 1, 8, &pixels)));
    assert!(parse_cursor_meta(&good).is_some());
    // Shorter than the cursor header; shorter than the pixels it names.
    assert!(parse_cursor_meta(&good[..20]).is_none());
    assert!(parse_cursor_meta(&good[..good.len() - 1]).is_none());
    // The bitmap block starts inside the cursor header or beyond the block.
    for start in [4u32, 27, 10_000] {
        let mut bad = good.clone();
        bad[24..28].copy_from_slice(&start.to_le_bytes());
        assert!(parse_cursor_meta(&bad).is_none(), "bitmap offset {start}");
    }
    // The pixel data starts inside the bitmap header or far beyond the block.
    for offset in [8u32, 1 << 30] {
        let mut bad = good.clone();
        bad[28 + 16..28 + 20].copy_from_slice(&offset.to_le_bytes());
        assert!(parse_cursor_meta(&bad).is_none(), "data offset {offset}");
    }
    // A row is wider than the stride, or the stride is negative.
    for stride in [4i32, -8] {
        let bad = block(1, (0, 0), (0, 0), Some((11, 2, 1, stride, &pixels)));
        assert!(parse_cursor_meta(&bad).is_none(), "stride {stride}");
    }
}

#[test]
fn bildkette_task_wayland_cursor_pointer_keeps_its_shape_while_it_moves_and_hides() {
    let mut pointer = Pointer::default();
    assert!(!pointer.delivers());
    assert!(pointer.apply(new_bitmap(1, 1, (0, 0), vec![1, 2, 3, 255])));
    assert!(pointer.delivers());
    assert_eq!(pointer.shown().map(|(_, at)| at), Some((0, 0)));
    // Only the position changed: the shape stays, nothing else changes.
    assert!(pointer.apply(moved(1, 9, 8)));
    assert_eq!(pointer.shown().map(|(_, at)| at), Some((9, 8)));
    assert!(!pointer.apply(moved(1, 9, 8)), "same place, same shape");
    // id 0 hides; the shape is kept for the next time.
    assert!(pointer.apply(moved(0, 9, 8)));
    assert!(pointer.shown().is_none());
    assert!(pointer.apply(moved(1, 2, 2)));
    assert!(pointer.shown().is_some());
    // An empty image removes the shape.
    let mut empty = moved(1, 2, 2);
    empty.bitmap = BitmapUpdate::Empty;
    assert!(pointer.apply(empty));
    assert!(pointer.shown().is_none());
}

#[test]
fn bildkette_task_wayland_cursor_route_overlay_or_mix_in() {
    // No pointer metadata: the pointer is in the picture (Embedded) or absent.
    assert_eq!(route(false, false), PointerRoute::InPicture);
    assert_eq!(route(true, false), PointerRoute::InPicture);
    // Metadata and a viewer with the overlay: exactly one pointer, the viewer's.
    assert_eq!(route(true, true), PointerRoute::Overlay);
    // Metadata and a viewer without it (0.5.196, window family, metadata
    // channel): the host mixes the pointer in, so it stays visible.
    assert_eq!(route(false, true), PointerRoute::Blend);
}

fn opaque(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    color.repeat((width * height) as usize)
}

#[test]
fn bildkette_task_wayland_cursor_blend_puts_the_pointer_at_its_hot_spot() {
    let size = FrameSize::new(5, 5);
    let mut picture = vec![100u8; size.bgra_len()];
    // 3 x 3 opaque red, hot spot in the middle, placed with its hot spot at (2, 2).
    let bitmap = PointerBitmap {
        width: 3,
        height: 3,
        hot_x: 1,
        hot_y: 1,
        rgba: opaque(3, 3, [255, 0, 0, 255]),
    };
    blend_premultiplied(&mut picture, size, &bitmap, 2, 2);
    let pixel = |x: usize, y: usize| picture[(y * 5 + x) * 4..(y * 5 + x) * 4 + 4].to_vec();
    assert_eq!(pixel(1, 1), [0, 0, 255, 255], "BGRA: red");
    assert_eq!(pixel(3, 3), [0, 0, 255, 255]);
    assert_eq!(pixel(0, 0), [100, 100, 100, 100], "outside the pointer");
    assert_eq!(pixel(4, 2), [100, 100, 100, 100]);
    // Half-transparent premultiplied green over 100: green = 128 + 100 * 127 / 255.
    let mut mixed = vec![100u8; size.bgra_len()];
    let green = PointerBitmap {
        width: 1,
        height: 1,
        hot_x: 0,
        hot_y: 0,
        rgba: vec![0, 128, 0, 128],
    };
    blend_premultiplied(&mut mixed, size, &green, 4, 4);
    assert_eq!(&mixed[(4 * 5 + 4) * 4..], [50, 178, 50, 255]);
    // A pointer partly or wholly outside the picture is clipped, never a panic.
    let mut edge = vec![100u8; size.bgra_len()];
    blend_premultiplied(&mut edge, size, &bitmap, 0, 0);
    assert_eq!(&edge[0..4], [0, 0, 255, 255]);
    blend_premultiplied(&mut edge, size, &bitmap, -50, 700);
    blend_premultiplied(&mut edge, size, &bitmap, i32::MIN, i32::MAX);
}

#[test]
fn bildkette_task_wayland_cursor_overlay_gets_shape_once_and_every_move() {
    let area = FrameSize::new(100, 50);
    let worker = WorkerCursor::new();
    worker.set_supported();
    assert!(worker.set_separate(true));
    let mut pointer = Pointer::default();
    let mut first = new_bitmap(2, 1, (1, 0), vec![255, 0, 0, 255, 0, 128, 0, 128]);
    first.x = 10;
    first.y = 20;
    pointer.apply(first);
    pointer.publish_overlay(&worker, area);
    let updates = worker.take();
    let [CursorUpdate::Shape(shape), CursorUpdate::Position(position)] = updates.as_slice() else {
        panic!("shape first, then position: {updates:?}");
    };
    assert_eq!(
        (shape.width, shape.height, shape.hot_x, shape.hot_y),
        (2, 1, 1, 0)
    );
    assert_eq!(&shape.rgba[4..8], [0, 128, 0, 128], "premultiplied RGBA");
    assert_eq!((position.x, position.y, position.visible), (10, 20, true));
    assert_eq!(position.shape_id, shape.id);
    assert_eq!(position.area, area);
    // Moved with the same shape: only the position, and nothing when it stays.
    pointer.apply(moved(1, 11, 20));
    pointer.publish_overlay(&worker, area);
    assert!(matches!(
        worker.take().as_slice(),
        [CursorUpdate::Position(p)] if p.x == 11
    ));
    pointer.publish_overlay(&worker, area);
    assert!(worker.take().is_empty());
    // Hidden (id 0) and outside the area: not visible for the viewer.
    pointer.apply(moved(0, 11, 20));
    pointer.publish_overlay(&worker, area);
    assert!(matches!(
        worker.take().as_slice(),
        [CursorUpdate::Position(p)] if !p.visible
    ));
    pointer.apply(moved(1, 500, 20));
    pointer.publish_overlay(&worker, area);
    assert!(matches!(
        worker.take().as_slice(),
        [CursorUpdate::Position(p)] if !p.visible
    ));
    // A new bitmap sends the shape again, with a new name.
    pointer.apply(new_bitmap(1, 1, (0, 0), vec![0, 0, 255, 255]));
    pointer.publish_overlay(&worker, area);
    let again = worker.take();
    let [CursorUpdate::Shape(next), CursorUpdate::Position(_)] = again.as_slice() else {
        panic!("{again:?}");
    };
    assert_ne!(next.id, shape.id);
}

#[test]
fn bildkette_task_wayland_cursor_feed_switches_the_overlay_on_with_the_first_metadata() {
    let size = FrameSize::new(64, 32);
    // The viewer asked for the overlay before the capture delivered any pointer:
    // the wish is kept and takes effect with the first metadata block.
    let worker = WorkerCursor::new();
    assert!(!worker.set_separate(true));
    let feed = PointerFeed::default();
    assert!(feed.step(&worker, size, None, None, None).is_none());
    assert!(
        !worker.separated(),
        "no pointer metadata yet: in the picture"
    );
    let mut first = new_bitmap(1, 1, (0, 0), vec![9, 9, 9, 255]);
    first.x = 5;
    first.y = 6;
    assert!(feed.step(&worker, size, Some(first), None, None).is_none());
    assert!(worker.separated());
    let updates = worker.take();
    assert!(
        matches!(
            updates.as_slice(),
            [CursorUpdate::Shape(_), CursorUpdate::Position(p)] if (p.x, p.y) == (5, 6)
        ),
        "{updates:?}"
    );
    // A viewer without the overlay never switches it on: nothing goes to it,
    // and without a picture to mix into nothing is published either.
    let plain = WorkerCursor::new();
    let other = PointerFeed::default();
    let meta = new_bitmap(1, 1, (0, 0), vec![9, 9, 9, 255]);
    assert!(other.step(&plain, size, Some(meta), None, None).is_none());
    assert!(!plain.separated());
    assert!(plain.take().is_empty());
}
