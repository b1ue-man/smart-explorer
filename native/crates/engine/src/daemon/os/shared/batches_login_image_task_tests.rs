//! Real IPC reader/viewer and delayed writer/registry fixtures for remote CI.
use super::image_fixture::{frame, image, login_image_fixture, pair, view};
use super::{registry, session};
use crate::{
    login::LoginCancellation,
    screen::core::{
        frame_memory::{CapacityCeiling, MemoryPool},
        image_admission::{ImageAdmission, ImageOwnerLimits, ImageRole, LocalImagePolicy},
        pixel_convert::FrameSize,
    },
};
use std::{
    io::{self, Write},
    sync::{mpsc, Arc, Condvar, Mutex},
    time::Duration,
};
fn admit(role: ImageRole, count: u64) -> Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(
        Arc::new(MemoryPool::default()),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(512 * 1024 * 1024),
        },
        ImageOwnerLimits::new(8)
            .unwrap()
            .with_role(role, 4, count)
            .unwrap(),
    )
}
#[test]
fn batches_task_login_image_productive_ipc_viewer_keeps_two_real_raw_slots() {
    let fixture = login_image_fixture(FrameSize::new(1, 1)).unwrap();
    let session = fixture.session.clone();
    let images = fixture.admission.clone();
    let first = session.wait_fixture_frame(Duration::from_secs(2)).unwrap();
    assert_eq!(fixture.next_capture(Duration::from_secs(1)).unwrap(), 1);
    session.visible();
    let second = session.wait_fixture_frame(Duration::from_secs(2)).unwrap();
    assert_eq!(fixture.next_capture(Duration::from_secs(1)).unwrap(), 2);
    assert_eq!(first.image.image().data(), &[1, 2, 3, 255]);
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 2);
    assert!(
        fixture.next_capture(Duration::from_millis(250)).is_err(),
        "externally held frames stop a third capture before allocation"
    );
    session.visible();
    session.release();
    assert!(
        !matches!(session.snapshot().state.as_str(), "ended" | "stopping"),
        "focus release without held input preserves the session"
    );
    fixture.finish().unwrap();
    assert_eq!(images.live_count(ImageRole::ViewerInput).unwrap(), 2);
    drop(first);
    drop(second);
    assert_eq!(images.live_bytes().unwrap(), 0);
}
struct Delayed {
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Option<mpsc::Sender<()>>,
}
struct ReleaseGate(Arc<(Mutex<bool>, Condvar)>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let mut released = self.0 .0.lock().unwrap();
        *released = true;
        self.0 .1.notify_all();
    }
}
impl Write for Delayed {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
            let lock = self.gate.0.lock().unwrap();
            drop(self.gate.1.wait_while(lock, |release| !*release).unwrap());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn batches_task_login_image_writer_completion_keeps_real_registry_owner() {
    let (_socket, server) = pair();
    let view = view();
    let id = view.session.clone();
    let owner =
        registry::register(view.clone(), Arc::new(LoginCancellation::default()), server).unwrap();
    let admission = admit(ImageRole::DecodedInput, 1);
    let image = image(&admission, ImageRole::DecodedInput, FrameSize::new(1, 1), 1);
    image.retain_lifetime(Arc::new(owner.clone())).unwrap();
    let frame = frame(view, 1, Some(image), "pixels");
    drop(owner);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let thread_gate = gate.clone();
    let _release_on_failure = ReleaseGate(gate.clone());
    let (started, ready) = mpsc::channel();
    let (complete, mut written) = tokio::sync::oneshot::channel();
    let worker = std::thread::spawn(move || {
        session::write_owned(
            &mut Delayed {
                gate: thread_gate,
                started: Some(started),
            },
            frame,
            complete,
        )
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(written.try_recv().is_err());
    assert!(registry::session(&id).unwrap().is_some());
    assert_eq!(admission.live_bytes().unwrap(), 4);
    {
        let mut released = gate.0.lock().unwrap();
        *released = true;
        gate.1.notify_all();
    }
    worker.join().unwrap().unwrap();
    written.blocking_recv().unwrap().unwrap();
    assert!(registry::session(&id).unwrap().is_none());
    assert_eq!(admission.live_bytes().unwrap(), 0);
}
