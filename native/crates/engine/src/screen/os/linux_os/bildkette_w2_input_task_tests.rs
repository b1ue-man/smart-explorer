//! Task suite „Bildkette“ merge W2 (Linux, X11): input of an X11 source has
//! its own endpoint and never waits behind a capture call (B-28, the BK3b
//! hand-off). Without an X server: the endpoint decides on the facts the
//! last capture published and refuses before it would touch a connection;
//! the waiting capture is a held lock standing for the worker lock. Not
//! registered yet (`docs/tests/register.tsv`): the manager adds the rows.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::screen::core::host_backend::ScreenInputSink;

fn area() -> Area {
    Area {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }
}

fn facts(reference: u64) -> InputFacts {
    InputFacts {
        reference,
        geometry: Some(area()),
        changed: false,
        image: true,
    }
}

#[test]
fn bildkette_task_linux_w2_x11_input_does_not_wait_behind_a_capture() {
    // The decision is `X11Source::inject_input`'s, on the published facts.
    let current = area();
    assert_eq!(admit(&facts(3), 3, true, current), Ok(()));
    assert_eq!(
        admit(&facts(3), 4, true, current),
        Err((ControlBlock::ControlUnavailable, false))
    );
    let moved = Area { x: 10, ..current };
    assert_eq!(
        admit(&facts(3), 3, true, moved),
        Err((ControlBlock::ControlUnavailable, true))
    );
    let no_picture = InputFacts {
        image: false,
        ..facts(3)
    };
    assert!(admit(&no_picture, 3, true, current).is_err());
    let changed = InputFacts {
        changed: true,
        ..facts(3)
    };
    assert!(admit(&changed, 3, false, current).is_err());
    assert_eq!(admit(&changed, 3, true, current), Ok(()));
    // A capture holds the worker lock for a second; the endpoint answers
    // at once (a stale reference here, so no X connection is needed).
    let worker = Arc::new(Mutex::new(()));
    let held = worker.clone();
    let (started, wait) = std::sync::mpsc::channel();
    let capture = std::thread::spawn(move || {
        let _guard = held.lock().unwrap();
        started.send(()).unwrap();
        std::thread::sleep(Duration::from_secs(1));
    });
    wait.recv().unwrap();
    let shared = Arc::new(SharedFacts::default());
    shared.publish(facts(7));
    let sink = X11Input {
        source: SourceId::Monitor("DP-1".into()),
        facts: shared.clone(),
    };
    let begun = Instant::now();
    let event = InputEvent::PointerAbs { x: 100, y: 100 };
    assert_eq!(
        sink.inject(&[event], 8, true),
        Err(ControlBlock::ControlUnavailable)
    );
    assert!(begun.elapsed() < Duration::from_millis(100));
    assert!(
        worker.try_lock().is_err(),
        "the capture still holds the lock"
    );
    capture.join().unwrap();
    // The endpoint reports a geometry change the source published.
    assert!(!sink.legacy_input_invalid());
    shared.publish(InputFacts {
        changed: true,
        ..facts(7)
    });
    assert!(sink.legacy_input_invalid());
    // It never names the capture's connection.
    let lane = include_str!("x11_input_lane.rs");
    assert!(!lane.contains("CaptureConnection") && !lane.contains(".conn"));
    let source = include_str!("x11_capture_source.rs");
    assert!(source.contains("fn input_sink(&self)"));
    assert!(source.contains("self.publish_input();"));
}
