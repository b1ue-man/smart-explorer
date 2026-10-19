//! Acceptance of the states that pause a Windows capture (SCR-M4): the order
//! of the checks with queries fed in, and the new capture session after a
//! pause on the secure desktop.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::screen::core::pixel_convert::BgraImage;

/// Answers of the queries, and the order they were asked in.
#[derive(Default)]
struct Fake {
    inactive: bool,
    secure: bool,
    gone: bool,
    minimized: bool,
    asked: Vec<&'static str>,
}

impl StateProbe for Fake {
    fn session_active(&mut self) -> bool {
        self.asked.push("session_active");
        !self.inactive
    }

    fn on_secure_desktop(&mut self) -> bool {
        self.asked.push("secure_desktop");
        self.secure
    }

    fn window_exists(&mut self) -> bool {
        self.asked.push("window_exists");
        !self.gone
    }

    fn window_minimized(&mut self) -> bool {
        self.asked.push("window_minimized");
        self.minimized
    }
}

fn state(window: bool, mut fake: Fake) -> (io::Result<Option<SessionState>>, Vec<&'static str>) {
    let state = state_before_capture(window, &mut fake);
    (state, fake.asked)
}

#[test]
fn screen_task_windows_session_state_order() {
    // Nothing pauses a monitor: two queries, none about a window.
    let (state_of, asked) = state(false, Fake::default());
    assert_eq!(state_of.unwrap(), None);
    assert_eq!(asked, ["session_active", "secure_desktop"]);
    // An inactive session ends the checks.
    let inactive = Fake {
        inactive: true,
        secure: true,
        ..Fake::default()
    };
    let (state_of, asked) = state(true, inactive);
    assert_eq!(state_of.unwrap(), Some(SessionState::SessionInactive));
    assert_eq!(asked, ["session_active"]);
    // The secure desktop comes before anything about the window.
    let secure = Fake {
        secure: true,
        gone: true,
        minimized: true,
        ..Fake::default()
    };
    let (state_of, asked) = state(true, secure);
    assert_eq!(state_of.unwrap(), Some(SessionState::SecureDesktop));
    assert_eq!(asked, ["session_active", "secure_desktop"]);
    // A window that is gone is an error, whether minimized or not.
    let gone = Fake {
        gone: true,
        minimized: true,
        ..Fake::default()
    };
    let (state_of, asked) = state(true, gone);
    let error = state_of.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "Das Fenster existiert nicht mehr");
    assert_eq!(asked, ["session_active", "secure_desktop", "window_exists"]);
    // A minimized window is asked for, after it was found to exist.
    let minimized = Fake {
        minimized: true,
        ..Fake::default()
    };
    let (state_of, asked) = state(true, minimized);
    assert_eq!(state_of.unwrap(), Some(SessionState::WindowMinimized));
    assert_eq!(
        asked,
        [
            "session_active",
            "secure_desktop",
            "window_exists",
            "window_minimized"
        ]
    );
    // A visible window is captured.
    let (state_of, asked) = state(true, Fake::default());
    assert_eq!(state_of.unwrap(), None);
    assert_eq!(asked.len(), 4);
    // The same checks for a monitor never ask about windows.
    let monitor = Fake {
        gone: true,
        minimized: true,
        ..Fake::default()
    };
    assert_eq!(state(false, monitor).0.unwrap(), None);
}

/// The answers of a probe that the test changes while the source runs.
#[derive(Clone, Default)]
struct Switches {
    secure: Arc<AtomicBool>,
    inactive: Arc<AtomicBool>,
}

impl StateProbe for Switches {
    fn session_active(&mut self) -> bool {
        !self.inactive.load(Ordering::SeqCst)
    }

    fn on_secure_desktop(&mut self) -> bool {
        self.secure.load(Ordering::SeqCst)
    }

    fn window_exists(&mut self) -> bool {
        true
    }

    fn window_minimized(&mut self) -> bool {
        false
    }
}

struct Counting {
    reopened: Arc<AtomicUsize>,
    captured: Arc<Mutex<Vec<bool>>>,
}

impl Grabber for Counting {
    fn size(&self) -> FrameSize {
        FrameSize::new(64, 64)
    }

    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        self.captured.lock().unwrap().push(keyframe);
        Ok(SourceFrame::Frame(BgraImage::new(FrameSize::new(64, 64))?))
    }

    fn reopen(&mut self) -> io::Result<()> {
        self.reopened.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn windows_capture_starts_anew_after_the_secure_desktop() {
    let reopened = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let switches = Switches::default();
    let grabber = Counting {
        reopened: reopened.clone(),
        captured: captured.clone(),
    };
    let mut source = WindowsSource::with(Box::new(grabber), switches.clone(), false);
    assert!(matches!(
        source.capture(1, false).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(reopened.load(Ordering::SeqCst), 0);
    // The lock screen: paused, nothing captured, and the session renewed
    // only once the desktop is back.
    switches.secure.store(true, Ordering::SeqCst);
    for frame_id in 2..5 {
        assert!(matches!(
            source.capture(frame_id, false).unwrap(),
            SourceFrame::Paused(SessionState::SecureDesktop)
        ));
    }
    assert_eq!(reopened.load(Ordering::SeqCst), 0);
    switches.secure.store(false, Ordering::SeqCst);
    assert!(matches!(
        source.capture(5, true).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(reopened.load(Ordering::SeqCst), 1);
    // An inactive session pauses the same way.
    switches.inactive.store(true, Ordering::SeqCst);
    assert!(matches!(
        source.capture(6, false).unwrap(),
        SourceFrame::Paused(SessionState::SessionInactive)
    ));
    switches.inactive.store(false, Ordering::SeqCst);
    assert!(matches!(
        source.capture(7, false).unwrap(),
        SourceFrame::Frame(_)
    ));
    assert_eq!(reopened.load(Ordering::SeqCst), 2);
    assert_eq!(*captured.lock().unwrap(), [false, true, false]);
}
