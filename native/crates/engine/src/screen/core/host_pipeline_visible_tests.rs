//! The visible area scaled to the picture the pipeline encodes
//! (`Pipeline::set_visible_rects`, `scale_visible`).
use super::*;
use crate::screen::core::visibility::VisibleRect;

/// A phone at 1344x2992 streamed as 448x998: the viewer reports the whole
/// picture it shows; scaling it to the same picture keeps it whole
/// (before, it was read as native pixels and shrank to a third).
#[test]
fn screen_metadata_runtime_task_visible_rects_use_the_picture_size() {
    let picture = FrameSize::new(448, 998);
    let whole = [VisibleRect {
        x: 0,
        y: 0,
        width: 448,
        height: 998,
    }];
    let scaled = scale_visible(picture, &whole, picture).expect("scaled");
    assert_eq!(scaled, whole.to_vec());
    let half = scale_visible(picture, &whole, FrameSize::new(224, 499)).expect("half");
    assert_eq!(
        half,
        vec![VisibleRect {
            x: 0,
            y: 0,
            width: 224,
            height: 499
        }]
    );
    assert!(scale_visible(FrameSize::new(0, 0), &whole, picture).is_none());
}

/// Round 12 (Codex 12/12): rectangles measured in the picture the viewer
/// shows keep that size until the pipeline applies them; a size change in
/// between scales them to the new picture instead of taking them for its
/// pixels (the whole 1000x500 picture was the upper left quarter of 2000x1000).
#[test]
fn r12_task_visible_area_keeps_its_basis_until_applied() {
    use crate::screen::core::visibility::{ViewerVisibility, VisibilityState, VisibleArea};
    let whole = vec![VisibleRect {
        x: 0,
        y: 0,
        width: 1000,
        height: 500,
    }];
    let mut report = ViewerVisibility {
        geometry_generation: 1,
        state: VisibilityState::Visible,
        minimized: false,
        rects: whole.clone(),
        picture: Some(FrameSize::new(1000, 500)),
    };
    // The host ran at 1000x500 at the report, at 2000x1000 when applying.
    let area = report.area(Some(FrameSize::new(1000, 500)));
    assert_eq!(
        area,
        VisibleArea {
            basis: Some(FrameSize::new(1000, 500)),
            rects: whole.clone(),
        }
    );
    let applied = scale_visible(area.basis.unwrap(), &area.rects, FrameSize::new(2000, 1000));
    assert_eq!(
        applied.expect("scaled"),
        vec![VisibleRect {
            x: 0,
            y: 0,
            width: 2000,
            height: 1000
        }]
    );
    // A viewer that names no picture: the size the host sent at the report.
    report.picture = None;
    assert_eq!(
        report.area(Some(FrameSize::new(1000, 500))).basis,
        Some(FrameSize::new(1000, 500))
    );
    assert_eq!(report.area(None).basis, None);
}
