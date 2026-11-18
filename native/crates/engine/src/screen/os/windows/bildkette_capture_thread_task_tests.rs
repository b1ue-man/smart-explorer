//! Task tests of plan 2026-11-14 bildkette, package BK2b (Windows capture):
//! the capture thread woken by the compositor (B-10), its frame interval
//! (B-11) and the monitor's switch to Graphics Capture (B-12). The Windows
//! runner has no desktop to capture: the native capture is a fake fed
//! through a channel, the secure desktop a switched probe.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::*;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SourceKey,
};
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::frame_budget::{FrameLimits, MAX_WORKING_BYTES};
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::os::capture_dxgi::wait::MAX_BLOCK;
use crate::screen::os::capture_prepared::MonitorGrabber;
use windows::Win32::Foundation::E_INVALIDARG;

const SIZE: FrameSize = FrameSize::new(8, 8);

/// A grant for `images` distinct 8 x 8 pictures.
fn grant(images: u64, name: &str) -> CaptureGrant {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            name,
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    let mut plan = CapturePlan::new(
        SourceKey::new(name, "bildkette-fixture").unwrap(),
        CaptureGeneration::new(1),
    );
    plan.add_bgra(CaptureOwner::SourcePixels, SIZE, images, CapturePhases::ALL)
        .unwrap();
    admission.authority().approve(plan).unwrap()
}

/// A compositor stand-in: `capture_waiting` waits for a picture number on
/// its channel and builds that picture; `capture` (a capture per request)
/// is counted, it must never run while the thread captures.
struct Compositor {
    pictures: Mutex<Receiver<u8>>,
    grant: CaptureGrant,
    released: Arc<Mutex<Vec<bool>>>,
    built: Arc<AtomicUsize>,
    waits: Arc<AtomicUsize>,
    direct: Arc<AtomicUsize>,
    intervals: Arc<Mutex<Vec<Duration>>>,
    path: CapturePath,
    honors_interval: bool,
    any: bool,
}

struct Handles {
    send: Sender<u8>,
    released: Arc<Mutex<Vec<bool>>>,
    built: Arc<AtomicUsize>,
    waits: Arc<AtomicUsize>,
    direct: Arc<AtomicUsize>,
    intervals: Arc<Mutex<Vec<Duration>>>,
}

fn compositor(images: u64, path: CapturePath, honors_interval: bool) -> (Compositor, Handles) {
    let (send, receive) = channel();
    let handles = Handles {
        send,
        released: Arc::default(),
        built: Arc::default(),
        waits: Arc::default(),
        direct: Arc::default(),
        intervals: Arc::default(),
    };
    let fake = Compositor {
        pictures: Mutex::new(receive),
        grant: grant(images, "bildkette-thread"),
        released: handles.released.clone(),
        built: handles.built.clone(),
        waits: handles.waits.clone(),
        direct: handles.direct.clone(),
        intervals: handles.intervals.clone(),
        path,
        honors_interval,
        any: false,
    };
    (fake, handles)
}

impl Grabber for Compositor {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        self.direct.fetch_add(1, Ordering::SeqCst);
        Ok(SourceFrame::Unchanged)
    }
    fn capture_waiting(
        &mut self,
        _keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourceFrame> {
        self.waits.fetch_add(1, Ordering::SeqCst);
        let picture = self.pictures.lock().unwrap().recv_timeout(wait);
        let Ok(number) = picture else {
            return Ok(if self.any {
                SourceFrame::Unchanged
            } else {
                SourceFrame::Paused(SessionState::Starting)
            });
        };
        self.released.lock().unwrap().push(release());
        let mut builder = CaptureImageBuilder::new(
            SIZE,
            FrameLimits::LEGACY,
            &self.grant,
            CaptureOwner::SourcePixels,
        )?;
        builder.pixels_mut()[0] = number;
        self.any = true;
        self.built.fetch_add(1, Ordering::SeqCst);
        Ok(SourceFrame::Frame(builder.freeze()))
    }
    fn reopen(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn set_frame_interval(&mut self, interval: Duration) -> bool {
        self.intervals.lock().unwrap().push(interval);
        self.honors_interval
    }
    fn path(&self) -> Option<CapturePath> {
        Some(self.path)
    }
}

/// A probe the test switches while the thread runs.
#[derive(Clone, Default)]
struct Probe {
    secure: Arc<AtomicBool>,
}

impl StateProbe for Probe {
    fn session_active(&mut self) -> bool {
        true
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

fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{what} did not happen");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn number(frame: SourceFrame) -> u8 {
    match frame {
        SourceFrame::Frame(image) => image.view().row(0)[0],
        SourceFrame::Unchanged => panic!("unchanged instead of a picture"),
        SourceFrame::Paused(state) => panic!("paused: {}", state.key()),
    }
}

#[test]
fn bildkette_task_win_capture_thread_requests_take_compositor_pictures() {
    // Room for two distinct pictures, as the capture plan admits (the one
    // the pipeline still holds and a new one).
    let (fake, handles) = compositor(2, CapturePath::DesktopDuplication, true);
    let mut grabber = thread::start(Box::new(fake), Box::new(Probe::default()), false);
    handles.send.send(1).unwrap();
    until("first picture", || {
        handles.built.load(Ordering::SeqCst) == 1
    });
    assert_eq!(number(grabber.capture(false).unwrap()), 1);
    // Nothing new: unchanged, or the newest again for a key frame.
    assert!(matches!(
        grabber.capture(false).unwrap(),
        SourceFrame::Unchanged
    ));
    // The request never captured itself.
    assert_eq!(handles.direct.load(Ordering::SeqCst), 0);
    assert!(handles.waits.load(Ordering::SeqCst) >= 1);
    // The pipeline keeps picture 1. Picture 2 is never taken: it is dropped
    // before picture 3 is read, otherwise three pictures would exist.
    let held = grabber.capture(true).unwrap();
    assert_eq!(number(held.clone_frame()), 1);
    handles.send.send(2).unwrap();
    until("second picture", || {
        handles.built.load(Ordering::SeqCst) == 2
    });
    handles.send.send(3).unwrap();
    until("third picture", || {
        handles.built.load(Ordering::SeqCst) == 3
    });
    assert_eq!(number(grabber.capture(false).unwrap()), 3);
    // First read: nothing before it; second: the one before was handed out;
    // third: the second was never taken, so its regions would be stale.
    assert_eq!(*handles.released.lock().unwrap(), [true, true, false]);
    drop(held);
    drop(grabber);
}

trait CloneFrame {
    fn clone_frame(&self) -> SourceFrame;
}

impl CloneFrame for SourceFrame {
    fn clone_frame(&self) -> SourceFrame {
        match self {
            SourceFrame::Frame(image) => SourceFrame::Frame(image.clone()),
            SourceFrame::Unchanged => SourceFrame::Unchanged,
            SourceFrame::Paused(state) => SourceFrame::Paused(*state),
        }
    }
}

#[test]
fn bildkette_task_win_capture_thread_first_request_waits_for_the_compositor() {
    let (fake, handles) = compositor(2, CapturePath::GraphicsCapture, true);
    let mut grabber = thread::start(Box::new(fake), Box::new(Probe::default()), false);
    let send = handles.send.clone();
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        send.send(7).unwrap();
    });
    // No picture yet: the request waits (up to the first-picture wait).
    assert_eq!(number(grabber.capture(false).unwrap()), 7);
    sender.join().unwrap();
    assert_eq!(handles.direct.load(Ordering::SeqCst), 0);
}

#[test]
fn bildkette_task_win_capture_thread_rests_on_the_secure_desktop() {
    let (fake, handles) = compositor(2, CapturePath::DesktopDuplication, true);
    let probe = Probe::default();
    let grabber = thread::start(Box::new(fake), Box::new(probe.clone()), false);
    let mut source = WindowsSource::with(grabber, probe.clone(), false);
    until("first wait", || handles.waits.load(Ordering::SeqCst) >= 1);
    probe.secure.store(true, Ordering::SeqCst);
    // One wait may be running when the desktop switches.
    std::thread::sleep(MAX_BLOCK * 2);
    let before = handles.waits.load(Ordering::SeqCst);
    std::thread::sleep(MAX_BLOCK * 3);
    assert_eq!(handles.waits.load(Ordering::SeqCst), before);
    assert!(matches!(
        source.capture(1, false).unwrap(),
        SourceFrame::Paused(SessionState::SecureDesktop)
    ));
    // Back on the user's desktop: the capture session is made anew (as
    // before B-10) and the next picture of the compositor is delivered.
    probe.secure.store(false, Ordering::SeqCst);
    assert!(matches!(
        source.capture(2, false).unwrap(),
        SourceFrame::Paused(SessionState::Starting)
    ));
    handles.send.send(4).unwrap();
    assert_eq!(number(source.capture(3, true).unwrap()), 4);
}

#[test]
fn bildkette_task_win_capture_thread_sets_the_frame_interval() {
    let (fake, handles) = compositor(2, CapturePath::GraphicsCapture, false);
    let mut grabber = thread::start(Box::new(fake), Box::new(Probe::default()), false);
    handles.send.send(1).unwrap();
    until("first picture", || {
        handles.built.load(Ordering::SeqCst) == 1
    });
    for _ in 0..6 {
        let _ = grabber.capture(false).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    until("interval", || !handles.intervals.lock().unwrap().is_empty());
    let interval = handles.intervals.lock().unwrap()[0];
    // The request cadence (10 ms plus scheduling), never a fixed rate.
    assert!(
        interval >= Duration::from_millis(5) && interval <= Duration::from_millis(60),
        "{interval:?}"
    );
}

/// A duplication stand-in whose `fallback_cause` the test sets.
struct Duplication {
    cause: Arc<Mutex<Option<FallbackCause>>>,
    reopen_error: Option<windows::core::HRESULT>,
}

impl Grabber for Duplication {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn reference_generation(&self) -> u64 {
        4
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
    fn reopen(&mut self) -> io::Result<()> {
        match self.reopen_error {
            Some(code) => Err(io::Error::other(windows::core::Error::from_hresult(code))),
            None => Ok(()),
        }
    }
    fn path(&self) -> Option<CapturePath> {
        Some(CapturePath::DesktopDuplication)
    }
    fn fallback_cause(&self) -> Option<FallbackCause> {
        *self.cause.lock().unwrap()
    }
}

struct Opened(u64);

impl Grabber for Opened {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn reference_generation(&self) -> u64 {
        self.0
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
    fn reopen(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn path(&self) -> Option<CapturePath> {
        Some(CapturePath::GraphicsCapture)
    }
}

#[test]
fn bildkette_task_win_capture_monitor_falls_back_to_graphics_capture() {
    use windows::Win32::Foundation::E_ACCESSDENIED;
    let cause = Arc::new(Mutex::new(None));
    let opened = Arc::new(AtomicUsize::new(0));
    let count = opened.clone();
    let mut monitor = MonitorGrabber {
        current: Box::new(Duplication {
            cause: cause.clone(),
            reopen_error: None,
        }),
        fallback: Some(Box::new(move |previous| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Opened(previous + 1)) as Box<dyn Grabber>)
        })),
        first: None,
        retry: None,
    };
    assert!(matches!(
        monitor.capture(false).unwrap(),
        SourceFrame::Unchanged
    ));
    assert_eq!(monitor.path(), Some(CapturePath::DesktopDuplication));
    assert_eq!(opened.load(Ordering::SeqCst), 0);
    // The duplication asks for the fallback: Graphics Capture with the next
    // generation, so the host process starts its encoder anew.
    *cause.lock().unwrap() = Some(FallbackCause::AccessLost);
    let _ = monitor
        .capture_waiting(false, Duration::ZERO, &mut || true)
        .unwrap();
    assert_eq!(monitor.path(), Some(CapturePath::GraphicsCapture));
    assert_eq!(monitor.reference_generation(), 5);
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    // A refused rebuild after the secure desktop pauses instead of ending.
    let mut paused = MonitorGrabber {
        current: Box::new(Duplication {
            cause: Arc::new(Mutex::new(None)),
            reopen_error: Some(E_ACCESSDENIED),
        }),
        fallback: None,
        first: None,
        retry: None,
    };
    assert!(paused.reopen().is_ok());
    let mut failed = MonitorGrabber {
        current: Box::new(Duplication {
            cause: Arc::new(Mutex::new(None)),
            reopen_error: Some(E_INVALIDARG),
        }),
        fallback: None,
        first: None,
        retry: None,
    };
    assert!(failed.reopen().is_err());
}

/// A first path whose opening the test decides.
struct Reopened(u64);

impl Grabber for Reopened {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn reference_generation(&self) -> u64 {
        self.0
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
    fn reopen(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn path(&self) -> Option<CapturePath> {
        Some(CapturePath::DesktopDuplication)
    }
}

#[test]
fn p9_task_win_capture_monitor_fallback_is_temporary() {
    use crate::screen::core::capture_fallback::{RetryPlan, RETRY_FIRST};
    let cause = Arc::new(Mutex::new(Some(FallbackCause::Failed)));
    let refuse = Arc::new(AtomicBool::new(true));
    let tries = Arc::new(AtomicUsize::new(0));
    let (refusing, counted) = (refuse.clone(), tries.clone());
    let mut monitor = MonitorGrabber {
        current: Box::new(Duplication {
            cause,
            reopen_error: None,
        }),
        fallback: Some(Box::new(|previous| {
            Ok(Box::new(Opened(previous + 1)) as Box<dyn Grabber>)
        })),
        first: Some(Box::new(move |previous| {
            counted.fetch_add(1, Ordering::SeqCst);
            if refusing.load(Ordering::SeqCst) {
                Err(io::Error::other(windows::core::Error::from_hresult(
                    windows::Win32::Graphics::Dxgi::DXGI_ERROR_UNSUPPORTED,
                )))
            } else {
                Ok(Box::new(Reopened(previous + 1)) as Box<dyn Grabber>)
            }
        })),
        retry: None,
    };
    // The duplication asks for the fallback: Graphics Capture, a retry plan.
    let _ = monitor.capture(false).unwrap();
    assert_eq!(monitor.path(), Some(CapturePath::GraphicsCapture));
    assert_eq!(monitor.reference_generation(), 5);
    assert!(monitor.retry.is_some());
    // Not yet due: no try.
    let _ = monitor.capture(false).unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 0);
    let past = Instant::now()
        .checked_sub(RETRY_FIRST)
        .expect("uptime of the test machine");
    // A due try that is refused: Graphics Capture again, after the try's
    // generation, and a longer wait.
    monitor.retry = Some(RetryPlan::new(past));
    let _ = monitor.capture(false).unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 1);
    assert_eq!(monitor.path(), Some(CapturePath::GraphicsCapture));
    assert_eq!(monitor.reference_generation(), 7);
    assert_eq!(monitor.retry.map(|plan| plan.wait()), Some(RETRY_FIRST * 2));
    // A due try that opens: Desktop Duplication, no plan left.
    refuse.store(false, Ordering::SeqCst);
    monitor.retry = Some(RetryPlan::new(past));
    let _ = monitor.capture(false).unwrap();
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    assert_eq!(monitor.path(), Some(CapturePath::DesktopDuplication));
    assert_eq!(monitor.reference_generation(), 8);
    assert!(monitor.retry.is_none());
}
