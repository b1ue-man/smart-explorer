//! Task suite „Bildkette“ BK3b (plan 2026-11-14 B-28, open point 1 of BK3):
//! input of a Windows source has its own way (`WindowsInput`): while a
//! capture call waits for the compositor and holds the source (and with it
//! the host process's worker), input is checked and answered without it.
//! Not registered yet (`docs/tests/register.tsv`): the manager adds the
//! rows.
use super::*;
use crate::screen::core::host_backend::ScreenSource;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const SIZE: FrameSize = FrameSize::new(64, 36);

/// A compositor that never delivers a picture.
struct Silent {
    generation: Arc<AtomicU64>,
}

impl Grabber for Silent {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn reference_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
    fn capture_waiting(
        &mut self,
        _keyframe: bool,
        wait: Duration,
        _release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourceFrame> {
        std::thread::sleep(wait);
        Ok(SourceFrame::Paused(SessionState::Starting))
    }
    fn reopen(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Desk;

impl StateProbe for Desk {
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

#[test]
fn bildkette_task_chain_input_does_not_wait_behind_a_waiting_capture() {
    let generation = Arc::new(AtomicU64::new(7));
    let grabber = thread::start(
        Box::new(Silent {
            generation: generation.clone(),
        }),
        Box::new(Desk),
        false,
    );
    let mut source = WindowsSource::with(grabber, Desk, false);
    let sink = source
        .input_sink()
        .expect("a Windows source has its input way");
    // The capture waits for the first picture of the compositor (up to
    // `FIRST_FRAME_WAIT`) and holds the source meanwhile.
    let capture = std::thread::spawn(move || {
        let started = Instant::now();
        let frame = source.capture(1, true).expect("capture");
        assert!(matches!(frame, SourceFrame::Paused(SessionState::Starting)));
        started.elapsed()
    });
    std::thread::sleep(Duration::from_millis(150));
    // Input for an older picture is refused at once, without the source.
    let started = Instant::now();
    let refused = sink.inject(&[], 6, true);
    assert_eq!(refused, Err(ControlBlock::ControlUnavailable));
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "input waited {:?}",
        started.elapsed()
    );
    let waited = capture.join().expect("capture thread");
    assert!(
        waited >= Duration::from_millis(500),
        "the capture did wait for the compositor ({waited:?})"
    );
}
