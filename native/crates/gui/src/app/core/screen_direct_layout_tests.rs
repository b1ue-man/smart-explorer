//! P8 (plan 2026-11-17): where the Direct3D 11 picture goes, which part of
//! it the video processor draws, and when egui's menus hide it.
use eframe::egui::{pos2, Order, Rect};

use super::*;

fn points(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::from_min_max(pos2(left, top), pos2(right, bottom))
}

#[test]
fn bildkette_task_d3d11_child_and_picture_follow_the_area() {
    // A 1600 × 900 point area below a 40 point toolbar at 1.5 pixels per
    // point; the picture letterboxed inside it.
    let area = points(0.0, 40.0, 1600.0, 940.0);
    let dest = points(100.0, 40.0, 1500.0, 940.0);
    let stats = points(1400.0, 48.0, 1592.0, 200.0);
    let geometry = geometry(area, dest, 1.5, &[stats, points(0.0, 0.0, 10.0, 10.0)]);
    assert_eq!(geometry.child, PixelRect::new(0, 60, 2400, 1410));
    assert_eq!(geometry.dest, PixelRect::new(150, 0, 2250, 1350));
    // The toolbar rectangle lies outside the area and is no cut-out.
    assert_eq!(geometry.cutouts, vec![PixelRect::new(2100, 12, 2388, 240)]);
    assert!(PixelRect::new(5, 5, 5, 9).is_empty());
}

#[test]
fn bildkette_task_d3d11_blit_rects_letterbox_and_one_to_one() {
    let source = FrameSize::new(1920, 1080);
    // Fit: the whole picture into its rectangle, black beside it.
    let fit = PixelRect::new(150, 0, 2250, 1350);
    assert_eq!(
        blit_rects(2400, 1350, fit, source),
        Some((PixelRect::new(0, 0, 1920, 1080), fit))
    );
    // 1:1 with pan: the picture is larger than the child and starts left of
    // and above it; only the visible part is drawn, pixel for pixel.
    let actual = PixelRect::new(-200, -100, 1720, 980);
    assert_eq!(
        blit_rects(1280, 720, actual, source),
        Some((
            PixelRect::new(200, 100, 1480, 820),
            PixelRect::new(0, 0, 1280, 720)
        ))
    );
    // Nothing visible, an empty picture: nothing to draw.
    assert_eq!(
        blit_rects(1280, 720, PixelRect::new(1300, 0, 2000, 400), source),
        None
    );
    assert_eq!(blit_rects(1280, 720, fit, FrameSize::new(0, 0)), None);
}

#[test]
fn bildkette_task_d3d11_menus_hide_overlays_cut_out() {
    let area = points(0.0, 40.0, 800.0, 600.0);
    let menu = (Order::Foreground, false, points(100.0, 30.0, 300.0, 200.0));
    let button = (Order::Foreground, true, points(700.0, 48.0, 790.0, 70.0));
    let tooltip = (Order::Tooltip, false, points(10.0, 50.0, 90.0, 70.0));
    let elsewhere = (Order::Foreground, false, points(0.0, 0.0, 50.0, 30.0));
    assert!(menu_over_picture(false, &[menu], area));
    assert!(menu_over_picture(true, &[], area), "an open combo box");
    assert!(!menu_over_picture(
        false,
        &[button, tooltip, elsewhere],
        area
    ));
}

#[test]
fn bildkette_task_d3d11_colour_space_of_stream_and_output() {
    // BT.709 limited range (se's hosts): matrix bit 2, nominal range 1 << 4.
    assert_eq!(colour_space_bits(Some((true, false))), 0b01_0100);
    assert_eq!(colour_space_bits(Some((false, false))), 0b01_0000);
    assert_eq!(colour_space_bits(Some((true, true))), 0b10_0100);
    // The swap chain: RGB 0–255, playback.
    assert_eq!(colour_space_bits(None), 0);
}
