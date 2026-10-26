use super::*;
use crate::screen::core::pixel_convert::FrameSize;
use eframe::egui::{pos2, vec2};

#[test]
fn batches_task_b6_visibility_projects_physical_rects_and_preserves_unknown() {
    let mut tracker = VisibilityTracker::default();
    let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 50.0));
    let mut geometry = Geometry {
        rect: area,
        size: FrameSize::new(1000, 500),
    };
    let mut window = WindowVisibility::default();
    let unknown = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert_eq!(unknown.state, VisibilityState::Unknown);
    assert_eq!(
        unknown.rects,
        vec![VisibleRect {
            x: 0,
            y: 0,
            width: 1000,
            height: 500
        }]
    );
    assert!(tracker
        .report(&window, area, Some(&geometry), 2.0)
        .is_none());
    window.shown = Some(true);
    window.rects = vec![Rect::from_min_max(pos2(40.0, 60.0), pos2(120.0, 100.0))];
    let visible = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert_eq!(visible.state, VisibilityState::Visible);
    assert_eq!(
        visible.rects,
        vec![VisibleRect {
            x: 100,
            y: 100,
            width: 400,
            height: 200
        }]
    );
    assert!(visible.geometry_generation > unknown.geometry_generation);
    geometry.size = FrameSize::new(2000, 1000);
    let resized = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert!(resized.geometry_generation > visible.geometry_generation);
    assert_eq!(
        resized.rects,
        vec![VisibleRect {
            x: 200,
            y: 200,
            width: 800,
            height: 400
        }]
    );
    window.shown = None;
    window.rects.clear();
    assert_eq!(
        tracker
            .report(&window, area, Some(&geometry), 2.0)
            .unwrap()
            .state,
        VisibilityState::Unknown
    );
    window.minimized = true;
    let hidden = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert!(hidden.hidden());
    assert!(hidden.rects.is_empty());
}

#[test]
fn batches_task_b6_resume_gate_requires_matching_generation_and_fresh_frame() {
    let mut gate = ResumeGate::default();
    let mut report = ViewerVisibility {
        geometry_generation: 1,
        state: VisibilityState::Visible,
        minimized: false,
        rects: vec![VisibleRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        }],
    };
    gate.report(&report, true);
    gate.complete(9);
    assert!(!gate.ready());
    gate.acknowledge(1, 10);
    assert!(!gate.can_present(9));
    gate.complete(10);
    assert!(gate.ready());
    report.geometry_generation = 2;
    report.state = VisibilityState::Hidden;
    gate.report(&report, true);
    gate.acknowledge(2, 10);
    gate.complete(100);
    assert!(!gate.ready());
    assert!(!gate.can_present(100));
    report.geometry_generation = 3;
    report.state = VisibilityState::Visible;
    gate.report(&report, true);
    gate.acknowledge(1, 0);
    assert!(!gate.can_present(100));
    gate.acknowledge(3, 101);
    gate.complete(100);
    assert!(!gate.ready());
    gate.complete(101);
    assert!(gate.ready());
    // A geometry change while visible also requires a fresh acknowledgement.
    report.geometry_generation = 4;
    report.rects[0].width = 20;
    gate.report(&report, true);
    assert!(!gate.can_present(101));
    gate.complete(102);
    gate.acknowledge(4, 102);
    assert!(gate.ready());
}
