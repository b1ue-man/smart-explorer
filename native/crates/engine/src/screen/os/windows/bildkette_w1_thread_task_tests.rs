//! Task tests of merge W1 (requests 5 and 6) on the capture thread: a key
//! frame of a picture whose NV12 ring was retired comes from a new capture
//! session instead of never; a still image of a GPU picture is the next
//! exact picture, on a still screen the first one of a new capture session.
//! The capture is a fake whose pictures the test queues; no GPU is needed.
use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_picture::{GpuPicture, GpuSurface};

const SIZE: FrameSize = FrameSize::new(16, 8);
/// The mark of the whole picture a new capture session delivers.
const NEW_SESSION: u8 = 99;

/// A ring slot the test retires.
struct Slot {
    gone: Arc<AtomicBool>,
}

impl GpuSurface for Slot {
    fn native(&self) -> FrameSize {
        SIZE
    }
    fn slot(&self) -> u64 {
        7
    }
    fn held(&self) -> bool {
        !self.gone.load(Ordering::SeqCst)
    }
    fn read_back(
        &self,
        _size: FrameSize,
        _limits: FrameLimits,
        _grant: &CaptureGrant,
        _owner: CaptureOwner,
    ) -> io::Result<CapturedImage> {
        Err(io::Error::from(io::ErrorKind::WouldBlock))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Gpu,
    Cpu(u8),
}

#[derive(Clone, Default)]
struct Handles {
    next: Arc<Mutex<VecDeque<Next>>>,
    reopened: Arc<AtomicUsize>,
    gone: Arc<AtomicBool>,
}

/// The pictures the test queued, one per wait; a new session (`reopen`)
/// queues the whole screen as a CPU picture marked `NEW_SESSION`.
struct Queued {
    handles: Handles,
    grant: CaptureGrant,
}

impl Grabber for Queued {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.grant.clone())
    }
    fn reference_generation(&self) -> u64 {
        self.handles.reopened.load(Ordering::SeqCst) as u64 + 1
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
    fn capture_waiting_picture(
        &mut self,
        _keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourcePicture> {
        let next = self.handles.next.lock().unwrap().pop_front();
        let picture = match next {
            None => {
                std::thread::sleep(wait.min(Duration::from_millis(2)));
                return Ok(SourcePicture::Unchanged);
            }
            Some(Next::Gpu) => {
                let slot = Arc::new(Slot {
                    gone: self.handles.gone.clone(),
                });
                Picture::Gpu(GpuPicture::new(
                    slot,
                    SIZE,
                    FrameLimits::LEGACY,
                    &self.grant,
                )?)
            }
            Some(Next::Cpu(mark)) => {
                let _ = release();
                let owner = CaptureOwner::SourcePixels;
                let mut image =
                    CaptureImageBuilder::new(SIZE, FrameLimits::LEGACY, &self.grant, owner)?;
                image.pixels_mut()[0] = mark;
                Picture::Cpu(image.freeze())
            }
        };
        Ok(SourcePicture::Frame(picture))
    }
    fn reopen(&mut self) -> io::Result<()> {
        self.handles.reopened.fetch_add(1, Ordering::SeqCst);
        let mut next = self.handles.next.lock().unwrap();
        next.clear();
        next.push_back(Next::Cpu(NEW_SESSION));
        Ok(())
    }
}

struct Probe;

impl StateProbe for Probe {
    fn session_active(&mut self) -> bool {
        true
    }
    fn on_secure_desktop(&mut self) -> bool {
        false
    }
    fn window_exists(&mut self) -> bool {
        true
    }
    fn window_minimized(&mut self) -> bool {
        false
    }
}

/// A capture thread whose first picture lies on the GPU, and that picture.
fn on_the_gpu() -> (Box<dyn Grabber>, Handles, SourcePicture) {
    let handles = Handles::default();
    handles.next.lock().unwrap().push_back(Next::Gpu);
    let fake = Queued {
        handles: handles.clone(),
        grant: crate::screen::core::capture_test_support::grant(SIZE, None),
    };
    let mut grabber = start(Box::new(fake), Box::new(Probe), false);
    let first = grabber.capture_picture(false).unwrap();
    assert!(matches!(first, SourcePicture::Frame(Picture::Gpu(_))));
    (grabber, handles, first)
}

fn mark(frame: SourceFrame) -> u8 {
    match frame {
        SourceFrame::Frame(image) => image.view().row(0)[0],
        SourceFrame::Unchanged => panic!("unchanged instead of a picture"),
        SourceFrame::Paused(state) => panic!("paused: {}", state.key()),
    }
}

#[test]
fn bildkette_task_win_w1_keyframe_of_a_retired_ring_captures_anew() {
    let (mut grabber, handles, _first) = on_the_gpu();
    // Still held: a key frame repeats the kept picture, no new session.
    let again = grabber.capture_picture(true).unwrap();
    assert!(matches!(again, SourcePicture::Frame(Picture::Gpu(_))));
    assert_eq!(handles.reopened.load(Ordering::SeqCst), 0);
    // The ring is retired (the GPU path failed) and the screen stands
    // still: the key frame's picture comes from a new capture session.
    handles.gone.store(true, Ordering::SeqCst);
    let SourcePicture::Frame(Picture::Cpu(image)) = grabber.capture_picture(true).unwrap() else {
        panic!("no picture for the key frame");
    };
    assert_eq!(image.view().row(0)[0], NEW_SESSION);
    assert_eq!(handles.reopened.load(Ordering::SeqCst), 1);
    // Requests without a key frame never renew the session.
    let next = grabber.capture_picture(false).unwrap();
    assert!(matches!(next, SourcePicture::Unchanged));
    assert_eq!(handles.reopened.load(Ordering::SeqCst), 1);
}

#[test]
fn bildkette_task_win_w1_still_image_takes_the_next_exact_picture() {
    let (mut grabber, handles, first) = on_the_gpu();
    // The screen changes: the next picture (read exactly) is the still
    // image, without a new capture session.
    let next = handles.next.clone();
    let feeder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        next.lock().unwrap().push_back(Next::Cpu(5));
    });
    assert_eq!(mark(grabber.exact_frame(first).unwrap()), 5);
    feeder.join().unwrap();
    assert_eq!(handles.reopened.load(Ordering::SeqCst), 0);
    // A CPU picture is exact already: no capture at all.
    let cpu = grabber.capture_picture(true).unwrap();
    assert_eq!(mark(grabber.exact_frame(cpu).unwrap()), 5);
}

#[test]
fn bildkette_task_win_w1_still_image_of_a_still_screen_renews_the_session() {
    let (mut grabber, handles, first) = on_the_gpu();
    // Nothing changes on the screen: after `MAX_BLOCK` a new capture
    // session delivers the whole picture, read exactly.
    let started = Instant::now();
    assert_eq!(mark(grabber.exact_frame(first).unwrap()), NEW_SESSION);
    assert!(started.elapsed() >= MAX_BLOCK);
    assert_eq!(handles.reopened.load(Ordering::SeqCst), 1);
}
