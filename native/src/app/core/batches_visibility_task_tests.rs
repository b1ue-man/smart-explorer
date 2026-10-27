use super::super::screen_session::{Picture, PictureData, Shared, ViewerSession};
use super::*;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
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

#[test]
fn screen_metadata_runtime_task_visibility_race_keeps_the_proven_report() {
    let mut tracker = VisibilityTracker::default();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 50.0));
    let geometry = Geometry {
        rect: area,
        size: FrameSize::new(200, 100),
    };
    let full = vec![VisibleRect {
        x: 0,
        y: 0,
        width: 200,
        height: 100,
    }];
    let mut window = WindowVisibility {
        shown: Some(true),
        rects: vec![Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0))],
        ..Default::default()
    };
    let proven = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert_eq!(proven.state, VisibilityState::Visible);
    assert_eq!(proven.rects, full);
    // A racing native sample (EnumWindows / stacking changed between its
    // two reads) with the same conservative pixels sends nothing.
    window.shown = None;
    window.rects.clear();
    assert!(tracker
        .report(&window, area, Some(&geometry), 2.0)
        .is_none());
    window.shown = Some(true);
    window.rects = vec![Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0))];
    assert!(tracker
        .report(&window, area, Some(&geometry), 2.0)
        .is_none());
    // A proven partial cover is reported; a race afterwards widens it to the
    // whole area conservatively (a real change of pixels).
    window.rects = vec![Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0))];
    let covered = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert_eq!(covered.state, VisibilityState::Visible);
    assert_eq!(
        covered.rects,
        vec![VisibleRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100
        }]
    );
    window.shown = None;
    window.rects.clear();
    let widened = tracker.report(&window, area, Some(&geometry), 2.0).unwrap();
    assert_eq!(widened.state, VisibilityState::Unknown);
    assert_eq!(widened.rects, full);
    // Minimizing still reports at once.
    window.minimized = true;
    assert!(tracker
        .report(&window, area, Some(&geometry), 2.0)
        .unwrap()
        .hidden());
}

fn runtime_picture(presented: super::super::screen_presented::PresentedFrame) -> Picture {
    Picture {
        seq: 1,
        presented,
        size: FrameSize::new(2, 2),
        data: PictureData::Bgra(BgraImage::new(FrameSize::new(2, 2)).unwrap()),
        memory: None,
    }
}

#[test]
fn screen_metadata_runtime_task_picture_before_visibility_ack_is_kept_not_dropped() {
    let shared = Shared::new(false, 0);
    shared.update(|info| info.state = SessionState::Streaming);
    let session = ViewerSession::fixture(shared.clone());
    let report = ViewerVisibility {
        geometry_generation: 1,
        state: VisibilityState::Visible,
        minimized: false,
        rects: Vec::new(),
    };
    // What `ViewerSession::set_visibility` does for a modern host.
    shared.invalidate_presentation();
    shared.resume.lock().unwrap().report(&report, true);
    let old = shared.begin_frame(1, 9);
    let fresh = shared.begin_frame(1, 10);
    assert!(!shared.can_present(&fresh));
    assert!(shared.awaiting_floor(&fresh));
    // The fresh picture overtook the acknowledgement: it waits in the slot.
    session.keep_if_awaiting_floor(runtime_picture(fresh.clone()));
    // A newer picture that arrived meanwhile is never replaced by it.
    let newer = shared.begin_frame(1, 11);
    *shared.picture.lock().unwrap() = Some(runtime_picture(newer.clone()));
    session.keep_if_awaiting_floor(runtime_picture(fresh.clone()));
    assert_eq!(
        shared.take_picture().map(|p| p.presented.frame_id),
        Some(11)
    );
    session.keep_if_awaiting_floor(runtime_picture(fresh.clone()));
    shared.visibility_ack(1, 10);
    assert!(!shared.awaiting_floor(&fresh));
    let kept = shared.take_picture().expect("the fresh picture was kept");
    assert!(shared.can_present(&kept.presented));
    assert!(!session.input_ready());
    (session.presented_callback(kept.presented.clone()))();
    // Without a HELLO frame contract the receipt stays local; input opens.
    assert!(session.input_ready());
    // A picture below the floor is still refused after the acknowledgement.
    assert!(!shared.can_present(&old));
    session.keep_if_awaiting_floor(runtime_picture(old));
    assert!(shared.take_picture().is_none());
}

#[test]
fn screen_metadata_runtime_task_new_picture_group_keeps_pixels_and_blocks_input() {
    let shared = Shared::new(false, 0);
    shared.update(|info| info.state = SessionState::Streaming);
    let session = ViewerSession::fixture(shared.clone());
    let first = shared.begin_frame(1, 1);
    (session.presented_callback(first.clone()))();
    assert!(session.input_ready());
    // A stream reset: the next key frame opens group 2.
    let second = shared.begin_frame(2, 2);
    assert!(first.active(), "the shown pixels stay until replaced");
    assert!(first.same_source(&second), "replaced in place, no blank");
    assert!(
        !shared.can_present(&first),
        "the old group gives no receipt"
    );
    assert!(!session.input_ready(), "input waits for the new group");
    assert!(shared.can_present(&second));
    (session.presented_callback(first.clone()))();
    assert!(!session.input_ready());
    (session.presented_callback(second.clone()))();
    assert!(session.input_ready());
    // A broken group keeps the pixels, ends their receipts, blocks input.
    shared.suspend_presentation();
    assert!(second.active());
    assert!(!shared.can_present(&second));
    assert!(!session.input_ready());
    // Visibility and source changes still revoke the pixels.
    shared.invalidate_presentation();
    assert!(!second.active());
}
